//! Lets the Chek web app, served from its own origin such as app.chek.page,
//! use this server from the browser. Apps open it signed in with a one-time
//! link code, so the session token never appears in a URL and people
//! signed in with a provider don't need a password.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderValue, Method, header};
use serde::{Deserialize, Serialize};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::warn;

use crate::api::{ApiError, AppState, blocking};
use crate::auth::{self, CurrentUser, SessionDto};
use crate::oidc::Expiring;

/// Long enough to switch to the browser, short enough that a link left in
/// history is useless.
const LINK_TTL: Duration = Duration::from_secs(60);

/// Link codes waiting to be opened, each naming the user it signs in.
#[derive(Default)]
pub struct WebLinks(Expiring<i64>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebLinkDto {
    code: String,
    /// Seconds until the code stops working.
    expires_in: u64,
}

/// Makes a code that signs the web app in as you, once.
pub async fn create_link(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<WebLinkDto>, ApiError> {
    let code = state.web_links.0.insert(LINK_TTL, current.user.id)?;
    Ok(Json(WebLinkDto {
        code,
        expires_in: LINK_TTL.as_secs(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OpenLinkRequest {
    code: String,
    /// A label for the session, such as "Chek on the web".
    device: Option<String>,
}

/// Trades a link code for a session of its own.
pub async fn open_link(
    State(state): State<AppState>,
    body: Result<Json<OpenLinkRequest>, JsonRejection>,
) -> Result<Json<SessionDto>, ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let user_id = state.web_links.0.take(&body.code).ok_or_else(|| {
        ApiError::bad_request(
            "the link has expired or was already used; open it from the app again",
        )
    })?;
    let user = blocking(&state, move |c| c.user(user_id))
        .await?
        .ok_or_else(|| ApiError::not_found("the account no longer exists".to_owned()))?;
    let device = body
        .device
        .map(|d| d.chars().take(auth::MAX_NAME_CHARS).collect::<String>());
    let token = auth::start_session(&state, user.id, device.as_deref()).await?;
    Ok(Json(SessionDto {
        token,
        user: user.into(),
    }))
}

/// Answers browsers on `origins`. Requests carry a bearer token, never
/// cookies, so another site can't act for someone just by sending them.
pub fn cors(origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| {
            HeaderValue::from_str(origin.trim_end_matches('/'))
                .inspect_err(|_| warn!(%origin, "ignored a web origin that isn't a valid header"))
                .ok()
        })
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(Duration::from_secs(600))
}
