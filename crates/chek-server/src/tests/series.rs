//! Renaming a series by hand, which scans keep.

use reqwest::StatusCode;
use serde_json::json;

use super::{RED_PNG, Server, assert_error, body, cbz};

#[tokio::test(flavor = "multi_thread")]
async fn series_edits_survive_rescans_until_cleared() {
    let server = Server::start().await;
    let admin = server.admin().await;
    let book = cbz(&[("01.png", &RED_PNG)]);
    let folder = server.folder(
        "comics",
        &[("Night Watch/01.cbz", &book), ("Night Watch/02.cbz", &book)],
    );
    let library = server.folder_library(&admin, "Comics", &folder).await;
    let reader = server.reader(&admin, "reader", &[library]).await;
    let publications = server.publications(&admin, library).await;
    let series = publications[0]["series"]["id"].as_i64().unwrap();
    assert_eq!(publications[0]["series"]["title"], "Night Watch");
    let path = format!("/api/v1/series/{series}");

    let response = server
        .patch(&path)
        .bearer_auth(&reader)
        .json(&json!({ "title": "Mine" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = server
        .patch(&path)
        .bearer_auth(&admin)
        .json(&json!({ "title": "  " }))
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::BAD_REQUEST, "bad_request").await;

    let response = server
        .patch(&path)
        .bearer_auth(&admin)
        .json(&json!({ "title": "Night Watch Omnibus" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let edited = body(response).await;
    assert_eq!(edited["title"], "Night Watch Omnibus");
    assert_eq!(edited["editedFields"], json!(["title"]));

    // The scan derives "Night Watch" from the folder again and the edit
    // is written back over it.
    let response = server
        .post(&format!("/api/v1/libraries/{library}/scan"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    server.wait_for_scan(&admin, library).await;
    let listed = body(
        server
            .get(&format!("/api/v1/libraries/{library}/series"))
            .bearer_auth(&reader)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(listed["items"][0]["title"], "Night Watch Omnibus");
    let found = body(
        server
            .get("/api/v1/search?q=Omnibus")
            .bearer_auth(&reader)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(found["items"].as_array().map(Vec::len), Some(2), "{found}");

    let response = server
        .delete(&format!("{path}/edits"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let restored = body(response).await;
    assert_eq!(restored["title"], "Night Watch");
    assert_eq!(restored["editedFields"], json!([]));
}
