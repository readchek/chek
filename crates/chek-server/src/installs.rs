//! Extensions each user installs from the repositories they can use.
//!
//! Installing downloads the component the repository's index lists, checks
//! its digest and that its manifest matches the index, and stores it under
//! the data folder. Installing again replaces the install with the version
//! the index lists now.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chek_catalog::ExtensionInstall;
use chek_sources::repository;
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, AppState, blocking};
use crate::auth::CurrentUser;
use crate::extensions::{parsed_index, repository_error, visible_repository};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledDto {
    id: String,
    name: String,
    version: String,
    repository_id: i64,
    sites: Vec<String>,
    languages: Vec<String>,
    installed_at: String,
    /// False while it came from the user's own repository and the admin
    /// allows only the allow list.
    usable: bool,
    /// The version the repository's index lists, when it differs from the
    /// installed one.
    update: Option<String>,
}

#[derive(Serialize)]
pub struct InstalledList {
    items: Vec<InstalledDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallRequest {
    repository_id: i64,
}

pub async fn list_installed(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<InstalledList>, ApiError> {
    let user_id = current.user.id;
    let (installs, repositories, allow_any) = blocking(&state, move |c| {
        Ok((
            c.extension_installs(user_id)?,
            c.extension_repositories(user_id)?,
            c.allow_any_repository()?,
        ))
    })
    .await?;
    let items = installs
        .into_iter()
        .map(|install| {
            let repository = repositories.iter().find(|r| r.id == install.repository_id);
            let usable = repository.is_some_and(|r| r.owner_id.is_none() || allow_any);
            let listed = repository
                .and_then(parsed_index)
                .and_then(|index| {
                    index
                        .extensions
                        .into_iter()
                        .find(|e| e.id == install.extension_id)
                })
                .map(|e| e.version);
            dto(install, usable, listed)
        })
        .collect::<Result<_, _>>()?;
    Ok(Json(InstalledList { items }))
}

/// Installs the extension `id` as the repository's index lists it now, or
/// updates an existing install to that version.
pub async fn install(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    body: Result<Json<InstallRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<InstalledDto>), ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let repository = visible_repository(&state, &current, body.repository_id).await?;
    let allow_any = blocking(&state, |c| c.allow_any_repository()).await?;
    if repository.owner_id.is_some() && !allow_any {
        return Err(not_usable());
    }
    let entry = parsed_index(&repository)
        .and_then(|index| index.extensions.into_iter().find(|e| e.id == id))
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "repository {} doesn't list an extension {id:?}",
                repository.id
            ))
        })?;

    let extensions = Arc::clone(&state.extensions);
    let index_url = repository.url.clone();
    let (digest, source) = tokio::task::spawn_blocking(move || {
        let bytes = repository::download(&index_url, &entry).map_err(repository_error)?;
        let digest = repository::sha256_hex(&bytes);
        let source = extensions
            .install(&bytes, &entry)
            .map_err(invalid_extension)?;
        Ok::<_, ApiError>((digest, source))
    })
    .await
    .map_err(ApiError::internal)??;

    let manifest = source.manifest();
    let install = ExtensionInstall {
        user_id: current.user.id,
        extension_id: manifest.id.clone(),
        repository_id: repository.id,
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        sha256: digest,
        sites: serde_json::to_string(&manifest.sites).map_err(ApiError::internal)?,
        languages: serde_json::to_string(&manifest.languages).map_err(ApiError::internal)?,
        installed_at: String::new(),
    };
    let user_id = install.user_id;
    let extension_id = install.extension_id.clone();
    let (existed, unused, saved) = blocking(&state, move |c| {
        let existed = c.extension_install(user_id, &extension_id)?.is_some();
        let unused = c.save_extension_install(&install)?;
        Ok((
            existed,
            unused,
            c.extension_install(user_id, &extension_id)?,
        ))
    })
    .await?;
    if let Some(digest) = unused {
        remove_file(&state, digest).await;
    }
    let saved = saved.ok_or_else(|| ApiError::internal("the install was just saved"))?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(dto(saved, true, None)?)))
}

pub async fn uninstall(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user_id = current.user.id;
    let lookup = id.clone();
    let removed = blocking(&state, move |c| {
        c.delete_extension_install(user_id, &lookup)
    })
    .await?
    .ok_or_else(|| not_installed(&id))?;
    if let (digest, true) = removed {
        remove_file(&state, digest).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes component files no install uses any more. A file left behind
/// only costs disk space, so failures are logged rather than returned.
pub(crate) async fn remove_file(state: &AppState, digest: String) {
    let extensions = Arc::clone(&state.extensions);
    let removed = tokio::task::spawn_blocking(move || extensions.remove(&digest)).await;
    if let Ok(Err(e)) = removed {
        tracing::warn!(error = %e, "couldn't delete an unused extension file");
    }
}

fn dto(
    install: ExtensionInstall,
    usable: bool,
    listed: Option<String>,
) -> Result<InstalledDto, ApiError> {
    Ok(InstalledDto {
        update: listed.filter(|v| *v != install.version),
        sites: serde_json::from_str(&install.sites).map_err(ApiError::internal)?,
        languages: serde_json::from_str(&install.languages).map_err(ApiError::internal)?,
        id: install.extension_id,
        name: install.name,
        version: install.version,
        repository_id: install.repository_id,
        installed_at: install.installed_at,
        usable,
    })
}

fn invalid_extension(error: chek_sources::Error) -> ApiError {
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: "extension_invalid",
        message: error.to_string(),
    }
}

fn not_usable() -> ApiError {
    ApiError {
        status: StatusCode::FORBIDDEN,
        code: "repository_not_allowed",
        message:
            "this repository isn't on the allow list, and the admin allows only the allow list"
                .to_owned(),
    }
}

fn not_installed(id: &str) -> ApiError {
    ApiError::not_found(format!("extension {id:?} isn't installed"))
}
