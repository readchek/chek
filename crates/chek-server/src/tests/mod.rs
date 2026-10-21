//! Tests that drive the server over HTTP, the way its clients do. Each test
//! starts its own server on a free port with a fresh catalog in a temporary
//! folder, so tests share nothing and run in parallel. They check behaviour
//! `openapi.yaml` promises, not how handlers are written.

mod accounts;
mod files;
mod progress;

use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chek_catalog::Catalog;
use chek_library::ScanOptions;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::api::{self, AppState};
use crate::{auth, oidc, remote};

pub(crate) const SETUP_TOKEN: &str = "test-setup-token";
pub(crate) const PASSWORD: &str = "correct horse battery";
/// Scans of a folder with a few files finish in milliseconds; this only
/// bounds a test that would otherwise hang.
const SCAN_WAIT: Duration = Duration::from_secs(30);

/// A 1×1 red PNG, small enough to compare whole and real enough for a cover.
pub(crate) const RED_PNG: [u8; 69] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xf7, 0x03, 0x41, 0x43, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];
/// A 1×1 blue PNG.
pub(crate) const BLUE_PNG: [u8; 69] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x60, 0x60, 0xf8, 0x0f,
    0x00, 0x01, 0x03, 0x01, 0x00, 0x36, 0x74, 0x11, 0x40, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

/// A running server and a client for it.
pub(crate) struct Server {
    base: String,
    http: Client,
    /// Holds the data folder, and any library folders a test makes in it,
    /// until the server is dropped.
    pub(crate) data: TempDir,
}

impl Server {
    pub(crate) async fn start() -> Self {
        Self::start_with(Vec::new()).await
    }

    /// Starts a server whose `demo_users` are shared demo accounts.
    pub(crate) async fn start_with(demo_users: Vec<String>) -> Self {
        let data = tempfile::tempdir().expect("a temporary folder");
        let catalog = Catalog::open(&data.path().join("chek.db")).expect("an empty catalog");
        let extensions = chek_sources::Extensions::new(
            data.path().join("extensions"),
            &chek_sources::Options {
                image_cache_bytes: 0,
                browser_solver: None,
                browser_executable: None,
                browser_args: Vec::new(),
                browser_native: false,
                worker_executable: None,
            },
        )
        .expect("an extension host");
        let state = AppState {
            catalog: Arc::new(catalog),
            scan: ScanOptions {
                workers: 2,
                covers_dir: data.path().join("covers"),
                http: chek_library::http_client().expect("an HTTP client"),
            },
            setup_token: Arc::default(),
            basic_auth_cache: Arc::default(),
            password_limiter: Arc::default(),
            trusted_proxies: Arc::new(Vec::new()),
            client_ip_header: None,
            max_upload_bytes: 1 << 20,
            extensions: Arc::new(extensions),
            progress_writes: remote::progress_writer(
                chek_library::http_client().expect("an HTTP client"),
            ),
            oidc: Arc::new(oidc::Oidc::new(None, Vec::new(), Vec::new())),
            demo_users: Arc::new(demo_users),
        };
        auth::prepare_setup(&state, Some(SETUP_TOKEN.to_owned()))
            .await
            .expect("the setup token");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free port");
        let address = listener.local_addr().expect("the bound address");
        let app = api::router(state).into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("the server runs");
        });
        Self {
            base: format!("http://{address}"),
            http: Client::new(),
            data,
        }
    }

    pub(crate) fn get(&self, path: &str) -> RequestBuilder {
        self.http.get(format!("{}{path}", self.base))
    }

    pub(crate) fn post(&self, path: &str) -> RequestBuilder {
        self.http.post(format!("{}{path}", self.base))
    }

    pub(crate) fn put(&self, path: &str) -> RequestBuilder {
        self.http.put(format!("{}{path}", self.base))
    }

    pub(crate) fn patch(&self, path: &str) -> RequestBuilder {
        self.http.patch(format!("{}{path}", self.base))
    }

    /// Creates the first admin, named `admin`, and returns their session token.
    pub(crate) async fn admin(&self) -> String {
        let response = self
            .post("/api/v1/setup")
            .json(&json!({ "setupToken": SETUP_TOKEN, "name": "admin", "password": PASSWORD }))
            .send()
            .await
            .expect("setup answers");
        assert_eq!(response.status(), StatusCode::CREATED);
        token(response).await
    }

    /// Creates a reader granted `libraries` and returns their session token.
    pub(crate) async fn reader(&self, admin: &str, name: &str, libraries: &[i64]) -> String {
        let response = self
            .post("/api/v1/users")
            .bearer_auth(admin)
            .json(&json!({
                "name": name,
                "password": PASSWORD,
                "role": "reader",
                "libraryIds": libraries,
            }))
            .send()
            .await
            .expect("create user answers");
        assert_eq!(response.status(), StatusCode::CREATED, "creating {name}");
        let response = self.sign_in(name, PASSWORD).await;
        assert_eq!(response.status(), StatusCode::OK, "signing in {name}");
        token(response).await
    }

    pub(crate) async fn sign_in(&self, name: &str, password: &str) -> Response {
        self.post("/api/v1/auth/sign-in")
            .json(&json!({ "name": name, "password": password }))
            .send()
            .await
            .expect("sign-in answers")
    }

    /// Makes a folder under the data folder holding `files`, each a name and
    /// its bytes, and returns its path.
    pub(crate) fn folder(&self, name: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let folder = self.data.path().join(name);
        std::fs::create_dir_all(&folder).expect("the library folder");
        for (file, bytes) in files {
            std::fs::write(folder.join(file), bytes).expect("the library file");
        }
        folder
    }

    /// Adds `folder` as a library and waits for its first scan to finish.
    pub(crate) async fn folder_library(&self, admin: &str, name: &str, folder: &Path) -> i64 {
        let response = self
            .post("/api/v1/libraries")
            .bearer_auth(admin)
            .json(&json!({ "name": name, "path": folder }))
            .send()
            .await
            .expect("add library answers");
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "adding library {name}"
        );
        let id = body(response).await["id"].as_i64().expect("a library id");
        self.wait_for_scan(admin, id).await;
        id
    }

    /// Waits until the library's scan is over.
    pub(crate) async fn wait_for_scan(&self, admin: &str, library: i64) {
        self.wait_until(|| async {
            let library = body(
                self.get(&format!("/api/v1/libraries/{library}"))
                    .bearer_auth(admin)
                    .send()
                    .await
                    .expect("get library answers"),
            )
            .await;
            library["scan"]["status"] == "idle" && !library["scan"]["finishedAt"].is_null()
        })
        .await;
    }

    /// Polls `done` until it's true, failing the test after [`SCAN_WAIT`].
    pub(crate) async fn wait_until<F, Fut>(&self, done: F)
    where
        F: Fn() -> Fut,
        Fut: Future<Output = bool>,
    {
        let waiting = async {
            while !done().await {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        tokio::time::timeout(SCAN_WAIT, waiting)
            .await
            .expect("the scan finishes in time");
    }

    /// The publications in a library, as the API lists them.
    pub(crate) async fn publications(&self, token: &str, library: i64) -> Vec<Value> {
        let response = self
            .get(&format!("/api/v1/libraries/{library}/publications"))
            .bearer_auth(token)
            .send()
            .await
            .expect("list publications answers");
        assert_eq!(response.status(), StatusCode::OK);
        body(response).await["items"]
            .as_array()
            .expect("a publication list")
            .clone()
    }
}

/// The response body as JSON.
pub(crate) async fn body(response: Response) -> Value {
    response.json().await.expect("a JSON body")
}

/// The session token in a sign-in or setup response.
async fn token(response: Response) -> String {
    body(response).await["token"]
        .as_str()
        .expect("a session token")
        .to_owned()
}

/// Checks an error response's status and `code`.
pub(crate) async fn assert_error(response: Response, status: StatusCode, code: &str) -> Value {
    assert_eq!(response.status(), status);
    let error = body(response).await;
    assert_eq!(error["code"], code, "{error}");
    error
}

/// A CBZ holding `entries`, each a name and its bytes, stored uncompressed
/// as most comic archivers do.
pub(crate) fn cbz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, bytes) in entries {
        archive.start_file(*name, options).expect("a zip entry");
        archive.write_all(bytes).expect("the entry's bytes");
    }
    archive.finish().expect("a finished zip").into_inner()
}

/// A Readium locator for `position` in a comic.
pub(crate) fn locator(position: u64) -> Value {
    json!({
        "href": format!("/pages/{}", position - 1),
        "type": "image/png",
        "locations": { "position": position, "totalProgression": 0.5 },
    })
}
