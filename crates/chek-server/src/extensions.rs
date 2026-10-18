//! Extension repositories and the extensions they offer.
//!
//! Repositories without an owner make up the admin's allow list, which
//! everyone sees. When an admin allows any repository, users may also add
//! their own, which only they see; turning the switch off again leaves
//! those in place but unusable.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chek_catalog::{ExtensionRepository, Role};
use chek_sources::repository::{self, Index, RepositoryError};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, AppState, blocking};
use crate::auth::CurrentUser;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDto {
    id: i64,
    url: String,
    name: Option<String>,
    /// On the admin's allow list rather than added by the user.
    allow_listed: bool,
    /// Whether its extensions can be installed and run. A user's own
    /// repository isn't while the admin allows only the allow list.
    usable: bool,
    extension_count: usize,
    fetched_at: Option<String>,
    fetch_error: Option<String>,
    created_at: String,
}

impl RepositoryDto {
    fn new(repository: ExtensionRepository, allow_any: bool) -> Self {
        let extension_count = parsed_index(&repository).map_or(0, |i| i.extensions.len());
        Self {
            id: repository.id,
            allow_listed: repository.owner_id.is_none(),
            usable: repository.owner_id.is_none() || allow_any,
            url: repository.url,
            name: repository.name,
            extension_count,
            fetched_at: repository.fetched_at,
            fetch_error: repository.fetch_error,
            created_at: repository.created_at,
        }
    }
}

#[derive(Serialize)]
pub struct RepositoryList {
    items: Vec<RepositoryDto>,
}

/// An extension a usable repository offers.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionDto {
    repository_id: i64,
    id: String,
    name: String,
    version: String,
    sites: Vec<String>,
    languages: Vec<String>,
}

#[derive(Serialize)]
pub struct ExtensionList {
    items: Vec<ExtensionDto>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsDto {
    allow_any_repository: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewRepository {
    url: String,
}

pub async fn list_repositories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<RepositoryList>, ApiError> {
    let user_id = current.user.id;
    let (repositories, allow_any) = blocking(&state, move |c| {
        Ok((
            c.extension_repositories(user_id)?,
            c.allow_any_repository()?,
        ))
    })
    .await?;
    Ok(Json(RepositoryList {
        items: repositories
            .into_iter()
            .map(|r| RepositoryDto::new(r, allow_any))
            .collect(),
    }))
}

/// Admins add to the allow list. Other users add a repository of their own,
/// which needs the admin to allow any repository.
pub async fn add_repository(
    State(state): State<AppState>,
    current: CurrentUser,
    body: Result<Json<NewRepository>, JsonRejection>,
) -> Result<(StatusCode, Json<RepositoryDto>), ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let url = repository::repository_url(&body.url).map_err(repository_error)?;
    let owner_id = (current.user.role != Role::Admin).then_some(current.user.id);
    let allow_any = blocking(&state, |c| c.allow_any_repository()).await?;
    if owner_id.is_some() && !allow_any {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "repository_not_allowed",
            message: "only an admin can add repositories while just the allow list is allowed"
                .to_owned(),
        });
    }
    let index = fetch(url.clone()).await?;
    let stored = serde_json::to_string(&index).map_err(ApiError::internal)?;
    let repository = blocking(&state, move |c| {
        c.add_extension_repository(&url, owner_id, index.name.as_deref(), &stored)
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(RepositoryDto::new(repository, allow_any)),
    ))
}

pub async fn delete_repository(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let repository = visible_repository(&state, &current, id).await?;
    if repository.owner_id.is_none() {
        current.require_admin()?;
    }
    blocking(&state, move |c| c.delete_extension_repository(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Fetches the repository's index again. A failed fetch is recorded on the
/// repository, which keeps its last good index, rather than failing the
/// request.
pub async fn refresh_repository(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<RepositoryDto>, ApiError> {
    let repository = visible_repository(&state, &current, id).await?;
    let fetched = match fetch(repository.url).await {
        Ok(index) => Ok((
            index.name.clone(),
            serde_json::to_string(&index).map_err(ApiError::internal)?,
        )),
        Err(e) => Err(e.message),
    };
    let (repository, allow_any) = blocking(&state, move |c| {
        let fetched = match &fetched {
            Ok((name, index)) => Ok((name.as_deref(), index.as_str())),
            Err(error) => Err(error.as_str()),
        };
        Ok((
            c.record_repository_fetch(id, fetched)?,
            c.allow_any_repository()?,
        ))
    })
    .await?;
    let repository = repository.ok_or_else(|| repository_not_found(id))?;
    Ok(Json(RepositoryDto::new(repository, allow_any)))
}

/// Every extension offered by the repositories the user may install from.
pub async fn list_extensions(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<ExtensionList>, ApiError> {
    let user_id = current.user.id;
    let (repositories, allow_any) = blocking(&state, move |c| {
        Ok((
            c.extension_repositories(user_id)?,
            c.allow_any_repository()?,
        ))
    })
    .await?;
    let items = repositories
        .iter()
        .filter(|r| r.owner_id.is_none() || allow_any)
        .filter_map(|r| Some((r.id, parsed_index(r)?)))
        .flat_map(|(repository_id, index)| {
            index.extensions.into_iter().map(move |e| ExtensionDto {
                repository_id,
                id: e.id,
                name: e.name,
                version: e.version,
                sites: e.sites,
                languages: e.languages,
            })
        })
        .collect();
    Ok(Json(ExtensionList { items }))
}

pub async fn get_settings(
    State(state): State<AppState>,
    _current: CurrentUser,
) -> Result<Json<SettingsDto>, ApiError> {
    let allow_any_repository = blocking(&state, |c| c.allow_any_repository()).await?;
    Ok(Json(SettingsDto {
        allow_any_repository,
    }))
}

pub async fn update_settings(
    State(state): State<AppState>,
    current: CurrentUser,
    body: Result<Json<SettingsDto>, JsonRejection>,
) -> Result<Json<SettingsDto>, ApiError> {
    current.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let allow = body.allow_any_repository;
    blocking(&state, move |c| c.set_allow_any_repository(allow)).await?;
    Ok(Json(body))
}

/// A repository on the allow list or one the user added; others answer as
/// not found.
pub(crate) async fn visible_repository(
    state: &AppState,
    current: &CurrentUser,
    id: i64,
) -> Result<ExtensionRepository, ApiError> {
    let user_id = current.user.id;
    blocking(state, move |c| c.extension_repository(id))
        .await?
        .filter(|r| r.owner_id.is_none_or(|owner| owner == user_id))
        .ok_or_else(|| repository_not_found(id))
}

/// The repository's stored index. It was checked when fetched, so one that
/// no longer reads is treated as empty.
pub(crate) fn parsed_index(repository: &ExtensionRepository) -> Option<Index> {
    repository::parse_index(repository.index.as_deref()?.as_bytes()).ok()
}

async fn fetch(url: String) -> Result<Index, ApiError> {
    tokio::task::spawn_blocking(move || repository::fetch_index(&url))
        .await
        .map_err(ApiError::internal)?
        .map_err(repository_error)
}

fn repository_error(error: RepositoryError) -> ApiError {
    let (status, code) = match error {
        RepositoryError::NotHttps(_) => (StatusCode::BAD_REQUEST, "bad_request"),
        RepositoryError::Fetch(_) | RepositoryError::Invalid(_) => {
            (StatusCode::BAD_GATEWAY, "repository_unreadable")
        }
    };
    ApiError {
        status,
        code,
        message: error.to_string(),
    }
}

fn repository_not_found(id: i64) -> ApiError {
    ApiError::not_found(format!("extension repository {id} does not exist"))
}
