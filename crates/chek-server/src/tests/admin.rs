//! What the admin panel relies on: server info before sign-in, the folder
//! picker, and renaming and deleting libraries.

use reqwest::StatusCode;
use serde_json::json;

use super::{RED_PNG, Server, assert_error, body, cbz};

#[tokio::test]
async fn server_info_says_when_setup_is_needed() {
    let server = Server::start().await;
    let info = body(server.get("/api/v1/server").send().await.unwrap()).await;
    assert_eq!(info["setupRequired"], true);
    assert!(info["version"].is_string());

    server.admin().await;
    let info = body(server.get("/api/v1/server").send().await.unwrap()).await;
    assert_eq!(info["setupRequired"], false);
}

#[tokio::test]
async fn admins_pick_library_folders_from_the_server() {
    let server = Server::start().await;
    let admin = server.admin().await;
    let root = server.folder("media", &[]);
    for name in ["Manga", "books", ".hidden"] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    std::fs::write(root.join("notes.txt"), "not a folder").unwrap();

    let response = server
        .get("/api/v1/server/folders")
        .query(&[("path", root.to_str().unwrap())])
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let listing = body(response).await;
    let root = root.canonicalize().unwrap();
    assert_eq!(listing["path"], root.to_str().unwrap());
    assert_eq!(listing["parent"], root.parent().unwrap().to_str().unwrap());
    assert_eq!(listing["truncated"], false);
    let names: Vec<_> = listing["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["books", "Manga"]);
    assert_eq!(
        listing["folders"][1]["path"],
        root.join("Manga").to_str().unwrap()
    );

    let response = server
        .get("/api/v1/server/folders")
        .query(&[("path", "relative")])
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::BAD_REQUEST, "bad_request").await;

    let reader = server.reader(&admin, "reader", &[]).await;
    let response = server
        .get("/api/v1/server/folders")
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn libraries_can_be_renamed_and_deleted() {
    let server = Server::start().await;
    let admin = server.admin().await;
    let folder = server.folder(
        "comics",
        &[("Saga/Saga 01.cbz", &cbz(&[("1.png", &RED_PNG)]))],
    );
    let library = server.folder_library(&admin, "Comics", &folder).await;
    let reader = server.reader(&admin, "reader", &[library]).await;

    let response = server
        .patch(&format!("/api/v1/libraries/{library}"))
        .bearer_auth(&admin)
        .json(&json!({ "name": "  Graphic novels " }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await["name"], "Graphic novels");

    let response = server
        .delete(&format!("/api/v1/libraries/{library}"))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = server
        .delete(&format!("/api/v1/libraries/{library}"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(folder.join("Saga/Saga 01.cbz").exists(), "files stay");

    let response = server
        .get(&format!("/api/v1/libraries/{library}"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::NOT_FOUND, "not_found").await;
    let search = body(
        server
            .get("/api/v1/search")
            .query(&[("q", "Saga")])
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(search["items"], json!([]));
    let me = body(
        server
            .get("/api/v1/me")
            .bearer_auth(&reader)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(me["libraryIds"], json!([]));
}
