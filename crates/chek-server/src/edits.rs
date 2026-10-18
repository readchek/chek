//! Hand edits to a publication's metadata. Admins only, since a library's
//! metadata is shared by everyone who reads it.

use std::time::Duration;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chek_catalog::{ContributorRecord, PublicationEdit};
use chek_formats::{MAX_FIELD_CHARS, MAX_SUMMARY_CHARS, Role};
use serde::{Deserialize, Deserializer};

use crate::api::{ApiError, AppState, blocking, publication_detail, start_scan};
use crate::auth::CurrentUser;

/// The most contributors a scan keeps from a file, and so the most an edit
/// may set.
const MAX_CONTRIBUTORS: usize = 50;

/// How long reverting waits for the file to be read again. A scan of the
/// same library holds that until it finishes.
const REREAD_WAIT: Duration = Duration::from_secs(20);

/// Fields left out stay as they are; `null` clears one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditRequest {
    #[serde(default, deserialize_with = "present")]
    title: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    number: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    description: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    publisher: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    language: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    published: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    reading_direction: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    contributors: Option<Option<Vec<ContributorRequest>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContributorRequest {
    name: String,
    role: String,
}

/// Tells a field sent as `null` apart from one left out.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// `PATCH /api/v1/publications/{id}`.
pub async fn edit_publication(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    body: Result<Json<EditRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    current.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let edit = checked(body)?;
    if !blocking(&state, move |c| c.edit_publication(id, &edit)).await? {
        return Err(ApiError::publication_not_found(id));
    }
    Ok(Json(publication_detail(&state, id).await?).into_response())
}

/// `DELETE /api/v1/publications/{id}/edits`: drops every edit and reads the
/// file's own metadata again.
pub async fn clear_edits(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    current.require_admin()?;
    if blocking(&state, move |c| c.publication(id))
        .await?
        .is_none()
    {
        return Err(ApiError::publication_not_found(id));
    }
    let Some(location) = blocking(&state, move |c| c.clear_edits(id)).await? else {
        // A source's publication is read again by its library's next sync.
        let library_id = blocking(&state, move |c| c.publication_library(id))
            .await?
            .ok_or_else(|| ApiError::publication_not_found(id))?;
        start_scan(&state, library_id);
        return Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "status": "queued" })),
        )
            .into_response());
    };
    let reread = tokio::spawn(chek_library::add_file(
        state.catalog.clone(),
        location.library_id,
        location.relative_path,
        state.scan.clone(),
    ));
    match tokio::time::timeout(REREAD_WAIT, reread).await {
        Err(_) => Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "status": "queued" })),
        )
            .into_response()),
        // A file that can't be read now keeps the metadata it had, without
        // edits; the next scan reports it like any other unreadable file.
        Ok(_) => Ok(Json(publication_detail(&state, id).await?).into_response()),
    }
}

fn checked(body: EditRequest) -> Result<PublicationEdit, ApiError> {
    let title = match body.title {
        None => None,
        Some(None) => return Err(ApiError::bad_request("title can't be cleared")),
        Some(Some(title)) => {
            let title = text("title", title, MAX_FIELD_CHARS)?
                .ok_or_else(|| ApiError::bad_request("title can't be empty"))?;
            let sort_title = chek_formats::natural_key(&title);
            Some((title, sort_title))
        }
    };
    let number = body
        .number
        .map(|n| {
            Ok::<_, ApiError>(optional_text("number", n, MAX_FIELD_CHARS)?.map(|label| {
                let sort_number = chek_formats::leading_number(&label);
                (label, sort_number)
            }))
        })
        .transpose()?;
    let published = body
        .published
        .map(|p| optional_text("published", p, 10))
        .transpose()?;
    if let Some(Some(date)) = &published
        && !is_partial_date(date)
    {
        return Err(ApiError::bad_request(
            "published must be YYYY, YYYY-MM or YYYY-MM-DD",
        ));
    }
    let reading_direction = body
        .reading_direction
        .map(|d| optional_text("readingDirection", d, 3))
        .transpose()?;
    if let Some(Some(direction)) = &reading_direction
        && direction != "ltr"
        && direction != "rtl"
    {
        return Err(ApiError::bad_request("readingDirection must be ltr or rtl"));
    }
    let contributors = body
        .contributors
        .map(|list| {
            let list = list.unwrap_or_default();
            if list.len() > MAX_CONTRIBUTORS {
                return Err(ApiError::bad_request(format!(
                    "a publication can list up to {MAX_CONTRIBUTORS} contributors"
                )));
            }
            list.into_iter()
                .map(|c| {
                    let name = text("contributor name", c.name, MAX_FIELD_CHARS)?
                        .ok_or_else(|| ApiError::bad_request("contributor names can't be empty"))?;
                    let role = Role::parse(&c.role).ok_or_else(|| {
                        ApiError::bad_request(format!(
                            "{:?} is not a contributor role; use author, illustrator, penciler, \
                             inker, colorist, letterer, coverArtist, editor, translator or contributor",
                            c.role
                        ))
                    })?;
                    Ok(ContributorRecord {
                        name,
                        role: role.as_str().to_owned(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(PublicationEdit {
        title,
        number,
        summary: body
            .description
            .map(|d| optional_text("description", d, MAX_SUMMARY_CHARS))
            .transpose()?,
        publisher: body
            .publisher
            .map(|p| optional_text("publisher", p, MAX_FIELD_CHARS))
            .transpose()?,
        language: body
            .language
            .map(|l| optional_text("language", l, 35))
            .transpose()?,
        published,
        reading_direction,
        contributors,
    })
}

/// Trims a text field; blank counts as cleared.
fn text(name: &str, value: String, max: usize) -> Result<Option<String>, ApiError> {
    let value = value.trim();
    if value.chars().count() > max {
        return Err(ApiError::bad_request(format!(
            "{name} is longer than {max} characters"
        )));
    }
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

fn optional_text(
    name: &str,
    value: Option<String>,
    max: usize,
) -> Result<Option<String>, ApiError> {
    value.map_or(Ok(None), |v| text(name, v, max))
}

fn is_partial_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let digits = |p: &str, n: usize| p.len() == n && p.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [y] => digits(y, 4),
        [y, m] => digits(y, 4) && digits(m, 2),
        [y, m, d] => digits(y, 4) && digits(m, 2) && digits(d, 2),
        _ => false,
    }
}
