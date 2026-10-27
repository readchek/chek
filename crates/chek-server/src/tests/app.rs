//! The web app served at the server's root: its screens, the reader's
//! frame with its own policy, and API addresses that stay the API's.

use reqwest::StatusCode;

use super::{Server, assert_error};

async fn server() -> (Server, tempfile::TempDir) {
    let build = tempfile::tempdir().unwrap();
    std::fs::write(
        build.path().join("index.html"),
        "<!doctype html><title>Chek</title>",
    )
    .unwrap();
    std::fs::create_dir_all(build.path().join("assets")).unwrap();
    std::fs::write(build.path().join("assets/index-abc.js"), "console.log(1)").unwrap();
    std::fs::create_dir_all(build.path().join("reader-frame")).unwrap();
    std::fs::write(
        build.path().join("reader-frame/frame.html"),
        "<!doctype html>",
    )
    .unwrap();
    (Server::start_with_app(build.path().to_owned()).await, build)
}

#[tokio::test]
async fn every_screen_gets_the_app() {
    let (server, _build) = server().await;
    for path in ["/", "/admin", "/admin/users", "/read/example.com/12"] {
        let response = server.get(path).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let headers = response.headers();
        assert_eq!(headers["cache-control"], "no-cache", "{path}");
        assert!(
            headers["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'none'")
        );
        assert_eq!(headers["x-frame-options"], "DENY");
        assert!(
            response
                .text()
                .await
                .unwrap()
                .contains("<title>Chek</title>"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn built_files_are_kept_and_the_frame_has_its_own_policy() {
    let (server, _build) = server().await;
    let script = server.get("/assets/index-abc.js").send().await.unwrap();
    assert_eq!(
        script.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );

    let frame = server.get("/reader-frame/frame.html").send().await.unwrap();
    let headers = frame.headers();
    let policy = headers["content-security-policy"].to_str().unwrap();
    assert!(policy.contains("frame-ancestors 'self'"));
    assert!(policy.contains("default-src 'none'"));
    assert!(!headers.contains_key("x-frame-options"));
}

#[tokio::test]
async fn api_addresses_stay_the_apis() {
    let (server, _build) = server().await;
    let response = server.get("/api/v1/nothing-here").send().await.unwrap();
    assert_error(response, StatusCode::NOT_FOUND, "not_found").await;
    let info = server.get("/api/v1/server").send().await.unwrap();
    assert_eq!(info.status(), StatusCode::OK);
    assert!(
        server
            .get("/health")
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
}

#[tokio::test]
async fn built_files_are_sent_compressed_when_the_browser_takes_it() {
    let (server, build) = server().await;
    std::fs::write(build.path().join("assets/index-abc.js.br"), "brotli").unwrap();
    std::fs::write(build.path().join("index.html.gz"), "gzip").unwrap();

    let script = server
        .get("/assets/index-abc.js")
        .header("accept-encoding", "gzip, br")
        .send()
        .await
        .unwrap();
    assert_eq!(script.headers()["content-encoding"], "br");
    assert!(
        script.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    // Caches in front of the server keep one copy per encoding.
    assert!(
        script.headers()["vary"]
            .to_str()
            .unwrap()
            .contains("accept-encoding")
    );
    assert_eq!(script.text().await.unwrap(), "brotli");

    // A screen's address gets the app's page, compressed the same way.
    let screen = server
        .get("/admin")
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(screen.headers()["content-encoding"], "gzip");
    assert_eq!(screen.text().await.unwrap(), "gzip");

    let plain = server.get("/assets/index-abc.js").send().await.unwrap();
    assert!(!plain.headers().contains_key("content-encoding"));
    assert_eq!(plain.text().await.unwrap(), "console.log(1)");
}
