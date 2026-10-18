//! Reading progress sync. Each save names the revision it builds on; when
//! another device saved in between, the save is refused with that device's
//! position so the reader can ask which one to keep. The server never picks
//! a winner by how far along each position is.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chek_catalog::{Progress, ProgressSave};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api::{ApiError, AppState, blocking, check_publication};
use crate::auth::CurrentUser;

/// Locators carry a short quote around the position, not whole chapters.
const MAX_LOCATOR_BYTES: usize = 16 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
    revision: i64,
    locator: Option<Value>,
    device: Option<String>,
    updated_at: Option<String>,
}

impl TryFrom<Progress> for ProgressDto {
    type Error = ApiError;

    fn try_from(p: Progress) -> Result<Self, ApiError> {
        let locator = p
            .locator
            .map(|l| serde_json::from_str(&l))
            .transpose()
            .map_err(ApiError::internal)?;
        Ok(Self {
            revision: p.revision,
            locator,
            device: p.device,
            updated_at: p.updated_at,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveProgressRequest {
    expected_revision: i64,
    locator: Value,
}

pub async fn get_progress(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<ProgressDto>, ApiError> {
    check_publication(&state, &current, id).await?;
    let user_id = current.user.id;
    let progress = blocking(&state, move |c| c.progress(user_id, id)).await?;
    Ok(Json(progress.try_into()?))
}

pub async fn save_progress(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    body: Result<Json<SaveProgressRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    if body.expected_revision < 0 {
        return Err(ApiError::bad_request(
            "expectedRevision is the revision last read, or 0 when there was none",
        ));
    }
    check_locator(&body.locator).map_err(ApiError::bad_request)?;
    let locator = body.locator.to_string();
    if locator.len() > MAX_LOCATOR_BYTES {
        return Err(ApiError::bad_request(format!(
            "the locator is larger than {MAX_LOCATOR_BYTES} bytes; shorten its text fields"
        )));
    }
    check_publication(&state, &current, id).await?;
    let user_id = current.user.id;
    let device = current.device;
    let expected = body.expected_revision;
    let saved = blocking(&state, move |c| {
        c.save_progress(user_id, id, expected, &locator, device.as_deref())
    })
    .await?;
    match saved {
        ProgressSave::Saved(progress) => Ok(Json(ProgressDto::try_from(progress)?).into_response()),
        ProgressSave::Conflict(progress) => {
            let body = serde_json::json!({
                "code": "progress_conflict",
                "message": format!(
                    "progress is at revision {}, not {expected}; \
                     choose a position and save again with that revision",
                    progress.revision
                ),
                "current": ProgressDto::try_from(progress)?,
            });
            Ok((StatusCode::CONFLICT, Json(body)).into_response())
        }
    }
}

/// Checks the fields of a Readium Locator that readers rely on. Other fields
/// are kept as sent.
fn check_locator(locator: &Value) -> Result<(), String> {
    let locator = locator
        .as_object()
        .ok_or("the locator must be a Readium Locator object")?;
    match locator.get("href").and_then(Value::as_str) {
        Some(href) if !href.is_empty() => {}
        _ => return Err("locator.href must be a non-empty string".to_owned()),
    }
    if !locator.get("type").is_some_and(Value::is_string) {
        return Err("locator.type must be the resource's media type".to_owned());
    }
    optional_string(locator, "title", "locator.title")?;
    if let Some(text) = locator.get("text") {
        let text = text.as_object().ok_or("locator.text must be an object")?;
        for field in ["before", "highlight", "after"] {
            optional_string(text, field, &format!("locator.text.{field}"))?;
        }
    }
    let Some(locations) = locator.get("locations") else {
        return Ok(());
    };
    let locations = locations
        .as_object()
        .ok_or("locator.locations must be an object")?;
    for field in ["progression", "totalProgression"] {
        if let Some(value) = locations.get(field) {
            match value.as_f64() {
                Some(p) if (0.0..=1.0).contains(&p) => {}
                _ => return Err(format!("locator.locations.{field} must be between 0 and 1")),
            }
        }
    }
    if let Some(position) = locations.get("position") {
        match position.as_u64() {
            Some(p) if p >= 1 => {}
            _ => {
                return Err("locator.locations.position must be an integer of 1 or more".to_owned());
            }
        }
    }
    if let Some(fragments) = locations.get("fragments") {
        let all_strings = fragments
            .as_array()
            .is_some_and(|f| f.iter().all(Value::is_string));
        if !all_strings {
            return Err("locator.locations.fragments must be an array of strings".to_owned());
        }
    }
    Ok(())
}

fn optional_string(object: &Map<String, Value>, field: &str, name: &str) -> Result<(), String> {
    match object.get(field) {
        None | Some(Value::String(_)) => Ok(()),
        Some(_) => Err(format!("{name} must be a string")),
    }
}
