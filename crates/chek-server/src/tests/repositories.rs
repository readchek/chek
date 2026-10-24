//! Extension repositories: who may add and change them, and the addresses
//! and credentials refused before the server fetches anything.

use reqwest::StatusCode;
use serde_json::json;

use super::{Server, assert_error, body};

#[tokio::test(flavor = "multi_thread")]
async fn readers_add_repositories_only_when_the_admin_allows() {
    let server = Server::start().await;
    let admin = server.admin().await;
    let reader = server.reader(&admin, "reader", &[]).await;

    let list = body(
        server
            .get("/api/v1/extension-repositories")
            .bearer_auth(&reader)
            .send()
            .await
            .unwrap(),
    )
    .await;
    let official = &list["items"][0];
    assert_eq!(official["url"], "https://chek.page/extensions/index.json");
    assert_eq!(official["allowListed"], true);
    assert_eq!(official["hasCredentials"], false);
    let official = official["id"].as_i64().unwrap();

    let add = |token: &str, request: serde_json::Value| {
        server
            .post("/api/v1/extension-repositories")
            .bearer_auth(token)
            .json(&request)
            .send()
    };
    assert_error(
        add(&reader, json!({ "url": "example.org/chek" }))
            .await
            .unwrap(),
        StatusCode::FORBIDDEN,
        "repository_not_allowed",
    )
    .await;

    let response = server
        .put("/api/v1/settings")
        .bearer_auth(&admin)
        .json(&json!({ "allowAnyRepository": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let settings = body(
        server
            .get("/api/v1/settings")
            .bearer_auth(&reader)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(settings["allowAnyRepository"], true);

    // Refused before anything is fetched: a password in the address would
    // be shown to everyone, and an empty header list can't sign in.
    assert_error(
        add(
            &reader,
            json!({ "url": "https://reader:secret@example.org/chek" }),
        )
        .await
        .unwrap(),
        StatusCode::BAD_REQUEST,
        "bad_request",
    )
    .await;
    assert_error(
        add(
            &reader,
            json!({ "url": "example.org/chek", "credentials": { "type": "headers", "headers": [] } }),
        )
        .await
        .unwrap(),
        StatusCode::BAD_REQUEST,
        "bad_request",
    )
    .await;

    // The allow list is the admin's to change.
    let response = server
        .patch(&format!("/api/v1/extension-repositories/{official}"))
        .bearer_auth(&reader)
        .json(&json!({ "credentials": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
