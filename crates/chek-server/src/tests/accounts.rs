//! Claiming the server, signing in, and what each account may see.

use reqwest::StatusCode;
use serde_json::json;

use super::{PASSWORD, RED_PNG, SETUP_TOKEN, Server, assert_error, body, cbz, locator};

#[tokio::test(flavor = "multi_thread")]
async fn setup_claims_the_server_once() {
    let server = Server::start().await;

    let response = server.get("/api/v1/me").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = server
        .post("/api/v1/setup")
        .json(&json!({ "setupToken": "guessed", "name": "intruder", "password": PASSWORD }))
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::FORBIDDEN, "forbidden").await;

    let admin = server.admin().await;
    let me = body(
        server
            .get("/api/v1/me")
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(me["name"], "admin");
    assert_eq!(me["role"], "admin");

    // The token is spent: nobody can claim the server a second time.
    let response = server
        .post("/api/v1/setup")
        .json(&json!({ "setupToken": SETUP_TOKEN, "name": "second", "password": PASSWORD }))
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::CONFLICT, "already_set_up").await;

    // Names aren't case-sensitive, as the contract says.
    let response = server.sign_in("Admin", PASSWORD).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = server.sign_in("admin", "wrong password").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_passwords_lock_the_address_out() {
    let server = Server::start().await;
    server.admin().await;

    for attempt in 1..=10 {
        let response = server.sign_in("admin", "wrong password").await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "attempt {attempt}"
        );
    }
    // Even the right password is refused now, so guessing can't continue.
    let response = server.sign_in("admin", PASSWORD).await;
    assert_error(response, StatusCode::TOO_MANY_REQUESTS, "too_many_attempts").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn readers_see_only_their_libraries() {
    let server = Server::start().await;
    let admin = server.admin().await;
    let book = cbz(&[("01.png", &RED_PNG)]);
    let mine = server.folder("mine", &[]);
    let theirs = server.folder("theirs", &[("book.cbz", &book)]);
    let mine = server.folder_library(&admin, "Mine", &mine).await;
    let theirs = server.folder_library(&admin, "Theirs", &theirs).await;
    let publication = server.publications(&admin, theirs).await[0]["id"]
        .as_i64()
        .unwrap();
    let reader = server.reader(&admin, "reader", &[mine]).await;

    let libraries = body(
        server
            .get("/api/v1/libraries")
            .bearer_auth(&reader)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(libraries["items"].as_array().unwrap().len(), 1);
    assert_eq!(libraries["items"][0]["id"], mine);

    // A library the reader wasn't granted looks like one that doesn't exist,
    // and so does everything in it.
    for path in [
        format!("/api/v1/libraries/{theirs}"),
        format!("/api/v1/libraries/{theirs}/publications"),
        format!("/api/v1/publications/{publication}"),
        format!("/api/v1/publications/{publication}/pages/0"),
        format!("/api/v1/publications/{publication}/progress"),
    ] {
        let response = server.get(&path).bearer_auth(&reader).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }

    // Readers can't run the server.
    let response = server
        .post("/api/v1/libraries")
        .bearer_auth(&reader)
        .json(&json!({ "name": "Sneaky", "path": server.data.path() }))
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::FORBIDDEN, "forbidden").await;
    let response = server
        .get("/api/v1/users")
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::FORBIDDEN, "forbidden").await;

    // A grant takes effect on the reader's existing session.
    let users = body(
        server
            .get("/api/v1/users")
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap(),
    )
    .await;
    let reader_id = users["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["name"] == "reader")
        .expect("the reader is listed")["id"]
        .as_i64()
        .unwrap();
    let response = server
        .patch(&format!("/api/v1/users/{reader_id}"))
        .bearer_auth(&admin)
        .json(&json!({ "libraryIds": [mine, theirs] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = server
        .get(&format!("/api/v1/publications/{publication}"))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn demo_accounts_change_nothing() {
    let server = Server::start_with(vec!["demo".to_owned()]).await;
    let admin = server.admin().await;
    let book = cbz(&[("01.png", &RED_PNG)]);
    let folder = server.folder("library", &[("book.cbz", &book)]);
    let library = server.folder_library(&admin, "Library", &folder).await;
    let publication = server.publications(&admin, library).await[0]["id"]
        .as_i64()
        .unwrap();
    let demo = server.reader(&admin, "demo", &[library]).await;

    // Nobody can lock the next visitor out by changing the password.
    let response = server
        .put("/api/v1/me/password")
        .bearer_auth(&demo)
        .json(&json!({ "currentPassword": PASSWORD, "newPassword": "my own password" }))
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::FORBIDDEN, "demo_account").await;
    assert_eq!(
        server.sign_in("demo", PASSWORD).await.status(),
        StatusCode::OK
    );

    // A reading position is answered as saved, since readers expect that,
    // but it never reaches the next visitor.
    let progress = format!("/api/v1/publications/{publication}/progress");
    let response = server
        .put(&progress)
        .bearer_auth(&demo)
        .json(&json!({ "expectedRevision": 0, "locator": locator(1) }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await["revision"], 1);
    let saved = body(
        server
            .get(&progress)
            .bearer_auth(&demo)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(saved["revision"], 0);
    assert!(saved["locator"].is_null(), "{saved}");
}

#[tokio::test(flavor = "multi_thread")]
async fn content_ratings_are_chosen_per_account() {
    let server = Server::start_with(vec!["demo".to_owned()]).await;
    let admin = server.admin().await;
    let reader = server.reader(&admin, "reader", &[]).await;
    let demo = server.reader(&admin, "demo", &[]).await;
    let ratings = |token: &str| {
        let request = server.get("/api/v1/me/content-ratings").bearer_auth(token);
        async move { body(request.send().await.unwrap()).await }
    };
    let set = |token: &str, chosen: serde_json::Value| {
        server
            .put("/api/v1/me/content-ratings")
            .bearer_auth(token)
            .json(&json!({ "contentRatings": chosen }))
            .send()
    };

    // Everyone starts out seeing nothing past fan service.
    let default = json!({ "contentRatings": ["safe", "suggestive"] });
    assert_eq!(ratings(&reader).await, default);

    // A choice comes back mildest first, without repeats, and is the
    // reader's alone.
    let response = set(&reader, json!(["pornographic", "safe", "safe"]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let chosen = json!({ "contentRatings": ["safe", "pornographic"] });
    assert_eq!(body(response).await, chosen);
    assert_eq!(ratings(&reader).await, chosen);
    assert_eq!(ratings(&admin).await, default);

    for invalid in [json!([]), json!(["nsfw"])] {
        let response = set(&reader, invalid).await.unwrap();
        assert_error(response, StatusCode::BAD_REQUEST, "bad_request").await;
    }
    assert_eq!(ratings(&reader).await, chosen);

    // Visitors to a demo account share it, so none chooses for the next.
    let response = set(&demo, json!(["safe"])).await.unwrap();
    assert_error(response, StatusCode::FORBIDDEN, "demo_account").await;
    assert_eq!(ratings(&demo).await, default);
}
