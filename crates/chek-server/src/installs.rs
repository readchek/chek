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
use crate::extensions::{credentials, parsed_index, repository_error, visible_repository};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledDto {
    id: String,
    name: String,
    version: String,
    repository_id: i64,
    sites: Vec<String>,
    languages: Vec<String>,
    /// What its chapters come in, `images`, `epub` or `pdf`, so clients
    /// know which readers it needs.
    formats: Vec<String>,
    /// The orders readers' comments come in, the site's default first, or
    /// empty when the source gives only a count. Absent when it gives none,
    /// or the admin hasn't turned comments on for its repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    comments: Option<Vec<String>>,
    installed_at: String,
    /// False while it came from the user's own repository and the admin
    /// allows only the allow list.
    usable: bool,
    /// The newest version the repository's index lists that this server
    /// runs, when it differs from the installed one.
    update: Option<String>,
    /// A newer version the index lists that needs a newer server, and the
    /// oldest Chek release that runs it.
    #[serde(skip_serializing_if = "Option::is_none")]
    blocked_update: Option<BlockedUpdate>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedUpdate {
    version: String,
    min_server: String,
}

/// What the index offers an install: the newest build this server runs,
/// and a newer one it can't run, if any.
fn offered(
    entry: Option<repository::IndexEntry>,
    installed: &str,
) -> (Option<String>, Option<BlockedUpdate>) {
    let Some(entry) = entry else {
        return (None, None);
    };
    let update = entry
        .build_for_server()
        .map(|b| b.version.clone())
        .filter(|v| v != installed);
    let blocked = match &entry.min_server {
        Some(min_server) if !entry.runs_here() && entry.version != installed => {
            Some(BlockedUpdate {
                version: entry.version.clone(),
                min_server: min_server.clone(),
            })
        }
        _ => None,
    };
    (update, blocked)
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
    let (installs, repositories, allow_any, comments_on) = blocking(&state, move |c| {
        Ok((
            c.extension_installs(user_id)?,
            c.extension_repositories(user_id)?,
            c.allow_any_repository()?,
            c.extension_comments()?,
        ))
    })
    .await?;
    let items = installs
        .into_iter()
        .map(|install| {
            let repository = repositories.iter().find(|r| r.id == install.repository_id);
            let usable = repository.is_some_and(|r| r.owner_id.is_none() || allow_any);
            let listed = repository.and_then(parsed_index).and_then(|index| {
                index
                    .extensions
                    .into_iter()
                    .find(|e| e.id == install.extension_id)
            });
            let offered = offered(listed, &install.version);
            let comments = comments_on && repository.is_some_and(|r| r.owner_id.is_none());
            dto(install, usable, offered, comments)
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
    current.refuse_demo()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let repository = visible_repository(&state, &current, body.repository_id).await?;
    install_for(&state, current.user.id, id, repository).await
}

/// Lets an admin provision a reader from a server-wide repository, including
/// a shared demo reader whose own requests cannot change its extensions.
pub async fn install_for_user(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((user_id, id)): Path<(i64, String)>,
    body: Result<Json<InstallRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<InstalledDto>), ApiError> {
    current.require_admin()?;
    current.refuse_demo()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let repository = blocking(&state, move |c| {
        if c.user(user_id)?.is_none() {
            return Ok(None);
        }
        Ok(c.extension_repository(body.repository_id)?
            .filter(|r| r.owner_id.is_none()))
    })
    .await?
    .ok_or_else(|| ApiError::not_found("user or server-wide repository not found".into()))?;
    install_for(&state, user_id, id, repository).await
}

async fn install_for(
    state: &AppState,
    user_id: i64,
    id: String,
    repository: chek_catalog::ExtensionRepository,
) -> Result<(StatusCode, Json<InstalledDto>), ApiError> {
    let allow_any = blocking(state, |c| c.allow_any_repository()).await?;
    if repository.owner_id.is_some() && !allow_any {
        return Err(not_usable());
    }
    let listed = parsed_index(&repository)
        .and_then(|index| index.extensions.into_iter().find(|e| e.id == id))
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "repository {} doesn't list an extension {id:?}",
                repository.id
            ))
        })?;
    let entry = match listed.build_for_server() {
        Some(build) => build.clone(),
        None => return Err(needs_newer_server(&listed)),
    };

    let extensions = Arc::clone(&state.extensions);
    let index_url = repository.url.clone();
    let credentials = credentials(&repository)?;
    let (digest, source) = tokio::task::spawn_blocking(move || {
        let bytes = repository::download(&index_url, &entry, credentials.as_ref())
            .map_err(repository_error)?;
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
        user_id,
        extension_id: manifest.id.clone(),
        repository_id: repository.id,
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        sha256: digest,
        sites: serde_json::to_string(&manifest.sites).map_err(ApiError::internal)?,
        languages: serde_json::to_string(&manifest.languages).map_err(ApiError::internal)?,
        formats: serde_json::to_string(
            &source
                .formats()
                .iter()
                .map(|f| chek_sources::format_name(*f))
                .collect::<Vec<_>>(),
        )
        .map_err(ApiError::internal)?,
        comments: source
            .comment_orders()
            .map(|orders| {
                serde_json::to_string(
                    &orders
                        .iter()
                        .map(|o| chek_sources::order_name(*o))
                        .collect::<Vec<_>>(),
                )
            })
            .transpose()
            .map_err(ApiError::internal)?,
        installed_at: String::new(),
    };
    let server_wide = repository.owner_id.is_none();
    let user_id = install.user_id;
    let extension_id = install.extension_id.clone();
    let (existed, unused, saved, comments_on) = blocking(state, move |c| {
        let existed = c.extension_install(user_id, &extension_id)?.is_some();
        let unused = c.save_extension_install(&install)?;
        Ok((
            existed,
            unused,
            c.extension_install(user_id, &extension_id)?,
            c.extension_comments()?,
        ))
    })
    .await?;
    if let Some(digest) = unused {
        remove_file(state, digest).await;
    }
    let saved = saved.ok_or_else(|| ApiError::internal("the install was just saved"))?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(dto(saved, true, (None, None), comments_on && server_wide)?),
    ))
}

pub async fn uninstall(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    current.refuse_demo()?;
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
    (update, blocked_update): (Option<String>, Option<BlockedUpdate>),
    comments_on: bool,
) -> Result<InstalledDto, ApiError> {
    let comments = match install.comments.as_deref() {
        Some(orders) if comments_on => {
            Some(serde_json::from_str(orders).map_err(ApiError::internal)?)
        }
        _ => None,
    };
    Ok(InstalledDto {
        comments,
        update,
        blocked_update,
        sites: serde_json::from_str(&install.sites).map_err(ApiError::internal)?,
        languages: serde_json::from_str(&install.languages).map_err(ApiError::internal)?,
        formats: serde_json::from_str(&install.formats).map_err(ApiError::internal)?,
        id: install.extension_id,
        name: install.name,
        version: install.version,
        repository_id: install.repository_id,
        installed_at: install.installed_at,
        usable,
    })
}

fn invalid_extension(error: chek_sources::Error) -> ApiError {
    match error {
        chek_sources::Error::Incompatible(_) => ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "extension_incompatible",
            message: format!(
                "this extension was built for another version of Chek's source contract; this server links {}",
                chek_sources::CONTRACT_VERSION
            ),
        },
        error @ chek_sources::Error::Mismatch(_) => ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "extension_mismatch",
            message: error.to_string(),
        },
        error => ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "extension_invalid",
            message: error.to_string(),
        },
    }
}

/// Every build the index lists needs a newer server than this one.
fn needs_newer_server(entry: &repository::IndexEntry) -> ApiError {
    let needed = entry.oldest_min_server().unwrap_or("a newer release");
    ApiError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        code: "extension_needs_newer_server",
        message: format!(
            "{} {} needs Chek {needed} or newer, and this server runs {}; update Chek to install it",
            entry.name,
            entry.version,
            repository::SERVER_VERSION
        ),
    }
}

pub(crate) fn not_usable() -> ApiError {
    ApiError {
        status: StatusCode::FORBIDDEN,
        code: "repository_not_allowed",
        message:
            "this repository isn't on the allow list, and the admin allows only the allow list"
                .to_owned(),
    }
}

pub(crate) fn not_installed(id: &str) -> ApiError {
    ApiError::not_found(format!("extension {id:?} isn't installed"))
}
