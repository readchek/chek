//! A Kavita library, synced from and served through a stand-in Kavita that
//! answers the few requests Chek makes, so the whole path from adding the
//! library to writing progress back runs without a Kavita install.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};

use super::{BLUE_PNG, RED_PNG, Server, assert_error, body, locator};

const API_KEY: &str = "kavita-key";
const TOKEN: &str = "kavita-session";

/// What the stand-in Kavita was told.
#[derive(Default)]
struct Received {
    progress: Vec<Value>,
}

type Stub = Arc<Mutex<Received>>;

/// Starts the stand-in Kavita and returns its address and what it receives.
async fn kavita() -> (String, Stub) {
    let received = Stub::default();
    let app = Router::new()
        .route("/api/Plugin/authenticate", post(authenticate))
        .route("/api/Library/libraries", get(libraries))
        .route("/api/Series/all-v2", post(series))
        .route("/api/Series/metadata", get(metadata))
        .route("/api/Series/volumes", get(volumes))
        .route("/api/Image/chapter-cover", get(cover))
        .route("/api/Reader/image", get(image))
        .route("/api/Reader/chapter-info", get(chapter_info))
        .route("/api/Reader/progress", post(progress))
        .layer(axum::middleware::from_fn(require_token))
        .with_state(received.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("the stand-in runs");
    });
    (format!("http://{address}"), received)
}

/// Everything but signing in needs the token signing in gave.
async fn require_token(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if request.uri().path() != "/api/Plugin/authenticate"
        && request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            != Some(&format!("Bearer {TOKEN}"))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

async fn authenticate(Query(query): Query<Value>) -> Response {
    if query["apiKey"] != API_KEY || query["pluginName"] != "Chek" {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({ "username": "chek", "token": TOKEN, "refreshToken": "r" })).into_response()
}

async fn libraries() -> Json<Value> {
    Json(json!([
        { "id": 1, "name": "Manga", "type": 0 },
        { "id": 2, "name": "Books", "type": 2 },
    ]))
}

async fn series(Query(query): Query<Value>) -> Json<Value> {
    // One page of results; the second page is empty.
    if query["PageNumber"] != "1" {
        return Json(json!([]));
    }
    Json(json!([
        { "id": 10, "name": "Blue Flag", "sortName": "Blue Flag", "libraryId": 1, "format": 1 },
        { "id": 20, "name": "A Novel", "sortName": "Novel, A", "libraryId": 2, "format": 3 },
    ]))
}

async fn metadata(Query(query): Query<Value>) -> Json<Value> {
    assert_eq!(query["seriesId"], "10", "only the manga library is read");
    Json(json!({
        "summary": "Taichi and Futaba.",
        "publishers": [{ "name": "Shueisha" }],
        "writers": [{ "name": "KAITO" }],
        "language": "en",
        "releaseYear": 2017,
    }))
}

async fn volumes(Query(query): Query<Value>) -> Json<Value> {
    assert_eq!(query["seriesId"], "10");
    let file = |name: &str| json!({ "filePath": format!("/manga/Blue Flag/{name}"), "pages": 2, "bytes": 1234, "format": 1 });
    Json(json!([
        {
            "id": 100, "name": "1", "minNumber": 1.0,
            "chapters": [{
                "id": 1000, "number": "0", "range": "0", "minNumber": 0.0, "pages": 2,
                "files": [file("Blue Flag v01.cbz")], "format": 1,
                "lastModifiedUtc": "2026-01-01T00:00:00", "releaseDate": "0001-01-01T00:00:00",
            }],
        },
        {
            "id": 101, "name": "-100000", "minNumber": -100000.0,
            "chapters": [{
                "id": 1001, "number": "9", "range": "9", "minNumber": 9.0, "pages": 1,
                "titleName": "The Ninth", "files": [file("Blue Flag c009.cbz")], "format": 1,
                "lastModifiedUtc": "2026-01-02T00:00:00", "releaseDate": "2018-03-04T00:00:00",
                "writers": [{ "name": "Someone Else" }],
            }],
        },
    ]))
}

async fn cover() -> Response {
    ([(CONTENT_TYPE, "image/png")], RED_PNG.to_vec()).into_response()
}

async fn image(Query(query): Query<Value>) -> Response {
    let page = match (query["chapterId"].as_str(), query["page"].as_str()) {
        (Some("1000"), Some("0")) => RED_PNG,
        (Some("1000"), Some("1")) => BLUE_PNG,
        (Some("1001"), Some("0")) => RED_PNG,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    ([(CONTENT_TYPE, "image/png")], page.to_vec()).into_response()
}

async fn chapter_info(Query(query): Query<Value>) -> Json<Value> {
    assert_eq!(query["chapterId"], "1000");
    Json(json!({ "volumeId": 100, "seriesId": 10, "libraryId": 1, "pages": 2 }))
}

async fn progress(State(received): State<Stub>, Json(body): Json<Value>) -> StatusCode {
    received.lock().unwrap().progress.push(body);
    StatusCode::OK
}

#[tokio::test(flavor = "multi_thread")]
async fn kavita_libraries_sync_serve_and_write_back() {
    let (kavita, received) = kavita().await;
    let server = Server::start().await;
    let admin = server.admin().await;

    // A wrong key is refused before the library exists.
    let response = server
        .post("/api/v1/libraries")
        .bearer_auth(&admin)
        .json(&json!({ "name": "Manga", "kavita": { "url": kavita, "apiKey": "wrong" } }))
        .send()
        .await
        .unwrap();
    let error = assert_error(response, StatusCode::BAD_REQUEST, "source_check_failed").await;
    assert!(
        error["message"].as_str().unwrap().contains("API key"),
        "{error}"
    );

    let response = server
        .post("/api/v1/libraries")
        .bearer_auth(&admin)
        .json(&json!({
            "name": "Manga",
            "kavita": { "url": kavita, "apiKey": API_KEY, "libraryIds": [1] },
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let library = body(response).await;
    assert_eq!(library["kind"], "source");
    assert_eq!(library["source"]["type"], "kavita");
    assert_eq!(library["source"]["libraryIds"], json!(["1"]));
    assert!(
        library["source"].get("apiKey").is_none(),
        "the key is never returned"
    );
    let library = library["id"].as_i64().unwrap();
    server.wait_for_scan(&admin, library).await;

    // Only the chosen Kavita library is read: two chapters of one series,
    // named the way Kavita shows them.
    let mut publications = server.publications(&admin, library).await;
    publications.sort_by_key(|p| p["title"].as_str().unwrap().to_owned());
    let titles: Vec<&str> = publications
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["The Ninth", "Volume 1"]);
    assert!(
        publications
            .iter()
            .all(|p| p["series"]["title"] == "Blue Flag" && p["format"] == "cbz"),
        "{publications:?}"
    );
    let volume = &publications[1];
    assert_eq!(volume["numberLabel"], "1");
    assert_eq!(volume["pageCount"], 2);
    assert_eq!(volume["hasCover"], true);
    let volume = volume["id"].as_i64().unwrap();

    let detail = body(
        server
            .get(&format!("/api/v1/publications/{volume}"))
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(detail["publisher"], "Shueisha");
    assert_eq!(detail["description"], "Taichi and Futaba.");
    assert_eq!(detail["language"], "en");
    assert_eq!(detail["published"], "2017");
    assert_eq!(
        detail["contributors"],
        json!([{ "name": "KAITO", "role": "author" }])
    );

    // Pages pass through from Kavita, with the type Kavita sends.
    let response = server
        .get(&format!("/api/v1/publications/{volume}/pages/1"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
    assert_eq!(response.bytes().await.unwrap().as_ref(), BLUE_PNG);

    // Finishing the volume reaches Kavita as its page count, against the
    // ids Kavita needs.
    let response = server
        .put(&format!("/api/v1/publications/{volume}/progress"))
        .bearer_auth(&admin)
        .json(&json!({ "expectedRevision": 0, "locator": locator(2) }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    server
        .wait_until(|| async { !received.lock().unwrap().progress.is_empty() })
        .await;
    let written = received.lock().unwrap().progress.clone();
    assert_eq!(
        written,
        [json!({
            "volumeId": 100, "chapterId": 1000, "pageNum": 2,
            "seriesId": 10, "libraryId": 1, "bookScrollId": null,
        })]
    );
}
