//! Reading progress sync between devices, and the OPDS sign-in challenge.

use reqwest::StatusCode;
use reqwest::header::WWW_AUTHENTICATE;
use serde_json::json;

use super::{PASSWORD, RED_PNG, Server, assert_error, body, cbz, locator};

#[tokio::test(flavor = "multi_thread")]
async fn progress_saves_name_the_revision_they_replace() {
    let server = Server::start().await;
    let admin = server.admin().await;
    let book = cbz(&[("01.png", &RED_PNG), ("02.png", &RED_PNG)]);
    let folder = server.folder("library", &[("book.cbz", &book)]);
    let library = server.folder_library(&admin, "Library", &folder).await;
    let id = server.publications(&admin, library).await[0]["id"]
        .as_i64()
        .unwrap();
    let progress = format!("/api/v1/publications/{id}/progress");
    // Two devices signed in to the same account.
    let phone = server.sign_in("admin", PASSWORD).await;
    let phone = body(phone).await["token"].as_str().unwrap().to_owned();

    let saved = body(
        server
            .get(&progress)
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(saved["revision"], 0);
    assert!(saved["locator"].is_null());

    let response = server
        .put(&progress)
        .bearer_auth(&admin)
        .json(&json!({ "expectedRevision": 0, "locator": locator(1) }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await["revision"], 1);

    // The phone hasn't seen revision 1, so its save is refused with the
    // position it would have overwritten, never merged by the server.
    let response = server
        .put(&progress)
        .bearer_auth(&phone)
        .json(&json!({ "expectedRevision": 0, "locator": locator(2) }))
        .send()
        .await
        .unwrap();
    let conflict = assert_error(response, StatusCode::CONFLICT, "progress_conflict").await;
    assert_eq!(conflict["current"]["revision"], 1);
    assert_eq!(conflict["current"]["locator"], locator(1));

    let response = server
        .put(&progress)
        .bearer_auth(&phone)
        .json(&json!({ "expectedRevision": 1, "locator": locator(2) }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let saved = body(
        server
            .get(&progress)
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(saved["revision"], 2);
    assert_eq!(saved["locator"], locator(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn opds_asks_for_basic_credentials() {
    let server = Server::start().await;
    server.admin().await;

    let response = server.get("/opds/v2/catalog").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let challenge = response.headers()[WWW_AUTHENTICATE].to_str().unwrap();
    assert!(challenge.starts_with("Basic realm=\"Chek\""), "{challenge}");

    let response = server
        .get("/opds/v2/catalog")
        .basic_auth("admin", Some("wrong password"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = server
        .get("/opds/v2/catalog")
        .basic_auth("admin", Some(PASSWORD))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
