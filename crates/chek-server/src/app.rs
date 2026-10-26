//! The Chek web app, served at the server's root when `CHEK_WEB_DIR` points
//! at its build, so a server on plain http, such as one at home, has a
//! reading app and an admin panel at `/admin` without app.chek.page. Every
//! address that isn't a file is one of the app's screens, so it gets the
//! app's page. Addresses under `/api` and `/opds` never do: a missing
//! endpoint is a 404, not the app.

use std::path::Path;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use tower_http::services::{ServeDir, ServeFile};

use crate::api::{ApiError, AppState};

/// The app's own pages: scripts and styles from this server only, and
/// connections to any https Chek server the reader signs in to.
const APP_POLICY: &str = "default-src 'self'; connect-src 'self' https:; img-src 'self' blob: data:; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The book reader's frame shows a publication's own pages. It runs in a
/// sandbox of its own origin, takes the book's styles, images and fonts as
/// blob URLs, runs only the reader's scripts, and reaches nothing on the
/// network. Only the app may frame it.
const FRAME_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'unsafe-inline' blob:; img-src blob: data:; font-src blob: data:; media-src blob:; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";

/// Serves the app's build from `dir`.
pub fn router(dir: &Path) -> Router<AppState> {
    let files = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
    Router::new()
        .route("/api/{*rest}", axum::routing::any(missing))
        .route("/opds/{*rest}", axum::routing::any(missing))
        .fallback_service(files)
        .layer(axum::middleware::from_fn(headers))
}

async fn missing(request: Request) -> ApiError {
    ApiError::not_found(format!("there's nothing at {}", request.uri().path()))
}

async fn headers(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    if path.starts_with("/api/") || path.starts_with("/opds/") {
        return response;
    }
    let headers = response.headers_mut();
    let html = headers
        .get(header::CONTENT_TYPE)
        .is_some_and(|t| t.as_bytes().starts_with(b"text/html"));
    // Built scripts and styles are named by their content, so they never
    // change; everything else is fetched fresh to pick up a new build.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if path.starts_with("/assets/") && !html {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    if path.starts_with("/reader-frame/") {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(FRAME_POLICY),
        );
    } else if html {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(APP_POLICY),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
