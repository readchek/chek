//! Bookmarks and highlights. Each belongs to one user; other users' and
//! publications the user can no longer read answer as not found. The last
//! save wins, since an annotation has no position to fight over.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chek_catalog::{Annotation, AnnotationChanges, AnnotationKind, NewAnnotation};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::api::{ApiError, AppState, blocking, check_publication};
use crate::auth::CurrentUser;
use crate::progress::{MAX_LOCATOR_BYTES, check_locator};

/// Far more than anyone marks in one book, and small enough to send in one
/// response.
const MAX_PER_PUBLICATION: i64 = 5_000;
const MAX_NOTE_CHARS: usize = 10_000;
const MAX_COLOR_CHARS: usize = 32;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationDto {
    id: i64,
    publication_id: i64,
    kind: &'static str,
    locator: Value,
    color: Option<String>,
    note: Option<String>,
    device: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<Annotation> for AnnotationDto {
    type Error = ApiError;

    fn try_from(a: Annotation) -> Result<Self, ApiError> {
        Ok(Self {
            id: a.id,
            publication_id: a.publication_id,
            kind: a.kind.as_str(),
            locator: serde_json::from_str(&a.locator).map_err(ApiError::internal)?,
            color: a.color,
            note: a.note,
            device: a.device,
            created_at: a.created_at,
            updated_at: a.updated_at,
        })
    }
}

#[derive(Serialize)]
pub struct AnnotationList {
    items: Vec<AnnotationDto>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum KindDto {
    Bookmark,
    Highlight,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewAnnotationRequest {
    kind: KindDto,
    locator: Value,
    color: Option<String>,
    note: Option<String>,
}

/// Fields left out stay as they are; `null` clears one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnnotationUpdateRequest {
    #[serde(default, deserialize_with = "present")]
    color: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    note: Option<Option<String>>,
}

/// Tells a field sent as `null` apart from one left out.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

pub async fn list_annotations(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<AnnotationList>, ApiError> {
    check_publication(&state, &current, id).await?;
    let user_id = current.user.id;
    let annotations = blocking(&state, move |c| c.annotations(user_id, id)).await?;
    Ok(Json(AnnotationList {
        items: annotations
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?,
    }))
}

pub async fn add_annotation(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    body: Result<Json<NewAnnotationRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<AnnotationDto>), ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let kind = match body.kind {
        KindDto::Bookmark => AnnotationKind::Bookmark,
        KindDto::Highlight => AnnotationKind::Highlight,
    };
    let locator = checked_locator(&body.locator)?;
    if kind == AnnotationKind::Highlight {
        let highlighted = body
            .locator
            .pointer("/text/highlight")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty());
        if !highlighted {
            return Err(ApiError::bad_request(
                "a highlight's locator.text.highlight must hold the highlighted text",
            ));
        }
    }
    let new = NewAnnotation {
        kind,
        locator,
        color: checked_color(body.color)?,
        note: checked_note(body.note)?,
    };
    check_publication(&state, &current, id).await?;
    let user_id = current.user.id;
    let device = current.device;
    let added = blocking(&state, move |c| {
        if c.annotation_count(user_id, id)? >= MAX_PER_PUBLICATION {
            return Ok(None);
        }
        c.add_annotation(user_id, id, &new, device.as_deref())
            .map(Some)
    })
    .await?
    .ok_or_else(|| {
        ApiError::bad_request(format!(
            "a publication can hold {MAX_PER_PUBLICATION} annotations per user; delete some first"
        ))
    })?;
    Ok((StatusCode::CREATED, Json(added.try_into()?)))
}

pub async fn update_annotation(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    body: Result<Json<AnnotationUpdateRequest>, JsonRejection>,
) -> Result<Json<AnnotationDto>, ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let changes = AnnotationChanges {
        color: body.color.map(checked_color).transpose()?,
        note: body.note.map(checked_note).transpose()?,
    };
    readable_annotation(&state, &current, id).await?;
    let user_id = current.user.id;
    let device = current.device;
    let updated = blocking(&state, move |c| {
        c.update_annotation(user_id, id, &changes, device.as_deref())
    })
    .await?
    .ok_or_else(|| annotation_not_found(id))?;
    Ok(Json(updated.try_into()?))
}

pub async fn delete_annotation(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    readable_annotation(&state, &current, id).await?;
    let user_id = current.user.id;
    if blocking(&state, move |c| c.delete_annotation(user_id, id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(annotation_not_found(id))
    }
}

/// Checks the annotation is the user's and in a publication they can still
/// read.
async fn readable_annotation(
    state: &AppState,
    current: &CurrentUser,
    id: i64,
) -> Result<(), ApiError> {
    let user_id = current.user.id;
    let annotation = blocking(state, move |c| c.annotation(user_id, id))
        .await?
        .ok_or_else(|| annotation_not_found(id))?;
    check_publication(state, current, annotation.publication_id)
        .await
        .map_err(|_| annotation_not_found(id))
}

fn checked_locator(locator: &Value) -> Result<String, ApiError> {
    check_locator(locator).map_err(ApiError::bad_request)?;
    let locator = locator.to_string();
    if locator.len() > MAX_LOCATOR_BYTES {
        return Err(ApiError::bad_request(format!(
            "the locator is larger than {MAX_LOCATOR_BYTES} bytes; shorten its text fields"
        )));
    }
    Ok(locator)
}

fn checked_color(color: Option<String>) -> Result<Option<String>, ApiError> {
    match color {
        Some(c) if c.is_empty() || c.chars().count() > MAX_COLOR_CHARS => Err(
            ApiError::bad_request(format!("color must be 1 to {MAX_COLOR_CHARS} characters")),
        ),
        color => Ok(color),
    }
}

fn checked_note(note: Option<String>) -> Result<Option<String>, ApiError> {
    match note {
        Some(n) if n.chars().count() > MAX_NOTE_CHARS => Err(ApiError::bad_request(format!(
            "note is longer than {MAX_NOTE_CHARS} characters"
        ))),
        note => Ok(note),
    }
}

fn annotation_not_found(id: i64) -> ApiError {
    ApiError::not_found(format!("annotation {id} does not exist"))
}
