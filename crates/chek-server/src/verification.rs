//! User-owned source browser handoff. No raw CDP endpoint reaches clients.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use base64::Engine;
use serde::Deserialize;

use crate::api::{ApiError, AppState};
use crate::auth::CurrentUser;
use crate::browse::usable_source;

pub async fn page() -> impl IntoResponse {
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src blob:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            ),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Html(include_str!("verification.html")),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Click {
    x: u32,
    y: u32,
}

pub async fn start(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<Start>,
) -> Result<StatusCode, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    tokio::task::spawn_blocking(move || source.start_handoff(&body.url))
        .await
        .map_err(ApiError::internal)?
        .map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn frame(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    let data = tokio::task::spawn_blocking(move || source.handoff_frame())
        .await
        .map_err(ApiError::internal)?
        .map_err(failed)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(ApiError::internal)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response())
}

pub async fn click(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<Click>,
) -> Result<StatusCode, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    tokio::task::spawn_blocking(move || source.handoff_click(body.x, body.y))
        .await
        .map_err(ApiError::internal)?
        .map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn close(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    tokio::task::spawn_blocking(move || source.close_handoff())
        .await
        .map_err(ApiError::internal)?
        .map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}

fn failed(message: String) -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "verification_unavailable",
        message,
    }
}

pub async fn resume(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    tokio::task::spawn_blocking(move || source.resume_handoff())
        .await
        .map_err(ApiError::internal)?
        .map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}
