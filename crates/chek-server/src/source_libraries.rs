//! Libraries that run on an extension: an admin makes one from an extension
//! they installed and follows series into it. Each chapter becomes a publication that readers
//! granted the library open like any other; its pages are fetched through
//! the extension when read.
//!
//! The library runs on the install of the admin who made it, so readers
//! don't need the extension themselves. If that admin uninstalls it, or its
//! repository stops being usable, the library's sources stop working until
//! it's installed again.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use chek_catalog::{ContributorRecord, RemoteChapter, RemoteContent, RemoteSeries};
use chek_library::{ExtensionSource, SourceConfig};
use chek_sources::{Image, ReadingDirection, Source};
use serde::Deserialize;
use tracing::{error, warn};

use crate::api::{ApiError, AppState, LibraryDto, SeriesDto, blocking, readable_series};
use crate::auth::CurrentUser;
use crate::browse::{image_from_json, image_json, run, usable_source};
use crate::installs::not_usable;

/// The `extension` field of a new library.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewExtensionSource {
    extension_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FollowRequest {
    series_id: String,
}

/// Makes a library that runs on an extension the admin installed. It starts
/// empty; series are followed into it one at a time.
pub(crate) async fn add_library(
    state: &AppState,
    current: &CurrentUser,
    name: String,
    extension: NewExtensionSource,
) -> Result<(StatusCode, Json<LibraryDto>), ApiError> {
    // Checks the extension is installed and usable now.
    usable_source(state, current, &extension.extension_id).await?;
    let config = serde_json::to_string(&SourceConfig::Extension(ExtensionSource {
        extension_id: extension.extension_id,
        user_id: current.user.id,
    }))
    .map_err(ApiError::internal)?;
    let library = blocking(state, move |c| c.add_source_library(&name, &config)).await?;
    Ok((StatusCode::CREATED, Json(library.into())))
}

/// Whether a library runs on an extension, so the server syncs it.
pub(crate) async fn is_extension_library(state: &AppState, library_id: i64) -> bool {
    matches!(
        blocking(state, move |c| c.library(library_id)).await,
        Ok(Some(library)) if library.source_config.as_deref().and_then(extension_config).is_some()
    )
}

/// Whether a publication is a chapter from an extension.
pub(crate) fn is_extension(remote: &RemoteContent) -> bool {
    extension_config(&remote.source_config).is_some()
}

fn extension_config(source_config: &str) -> Option<ExtensionSource> {
    match serde_json::from_str(source_config) {
        Ok(SourceConfig::Extension(source)) => Some(source),
        _ => None,
    }
}

/// Follows a series into a source library, adding its chapters.
pub async fn follow_series(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(library_id): Path<i64>,
    body: Result<Json<FollowRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<SeriesDto>), ApiError> {
    current.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let source = library_source(&state, library_id).await?;
    let series_id = record_series(&state, library_id, source, body.series_id).await?;
    let series = readable_series(&state, &current, series_id).await?;
    Ok((StatusCode::CREATED, Json(series.into())))
}

/// Updates every series the library follows, recording progress in the
/// library's scan status like a folder scan.
pub(crate) async fn sync(state: &AppState, library_id: i64) {
    match blocking(state, move |c| c.begin_scan(library_id)).await {
        Ok(true) => {}
        Ok(false) | Err(_) => return,
    }
    let failure = sync_series(state, library_id).await.err();
    if let Some(message) = &failure {
        error!(library_id, error = %message, "source sync failed");
    }
    let _ = blocking(state, move |c| {
        c.finish_scan(library_id, failure.as_deref())
    })
    .await;
}

/// Fails with what to show as the library's scan error.
async fn sync_series(state: &AppState, library_id: i64) -> Result<(), String> {
    let source = library_source(state, library_id)
        .await
        .map_err(|e| e.message)?;
    let remote_ids = blocking(state, move |c| c.remote_series_ids(library_id))
        .await
        .map_err(|e| e.message)?;
    let mut failed = 0;
    for remote_id in remote_ids {
        if let Err(e) =
            record_series(state, library_id, Arc::clone(&source), remote_id.clone()).await
        {
            warn!(library_id, remote_id, error = %e.message, "couldn't update a followed series");
            failed += 1;
        }
    }
    if failed > 0 {
        return Err(format!("{failed} series couldn't be updated; see the log"));
    }
    Ok(())
}

/// The media types of a chapter's pages, guessed from their URLs, fetching
/// the page list from the source when none is stored.
pub(crate) async fn page_types(
    state: &AppState,
    remote: RemoteContent,
) -> Result<Vec<&'static str>, ApiError> {
    let pages = pages(state, &remote, false).await?;
    Ok(pages
        .iter()
        .map(|page| {
            chek_formats::page_media_type(page.url.split(['?', '#']).next().unwrap_or_default())
        })
        .collect())
}

/// One page of a chapter, fetched through the source. Sources often hand
/// out page links that expire, so a failed fetch asks the source for the
/// page list again and retries once.
pub(crate) async fn get_page(
    state: &AppState,
    remote: RemoteContent,
    index: u32,
) -> Result<Response, ApiError> {
    let id = remote.publication_id;
    let source = library_source(state, remote.library_id).await?;
    let missing = || ApiError::not_found(format!("publication {id} has no page {index}"));
    let stored = pages(state, &remote, false).await?;
    let page = stored.get(index as usize).cloned().ok_or_else(missing)?;
    let fetched = match run(Arc::clone(&source), move |s| s.fetch_image(&page)).await {
        Ok(fetched) => fetched,
        Err(_) => {
            let fresh = pages(state, &remote, true).await?;
            let page = fresh.get(index as usize).cloned().ok_or_else(missing)?;
            run(source, move |s| s.fetch_image(&page)).await?
        }
    };
    let content_type = fetched
        .content_type
        .filter(|t| t.starts_with("image/"))
        .and_then(|t| HeaderValue::from_str(&t).ok())
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));
    let mut response = Response::new(Body::from(fetched.bytes));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

/// The stored page list, or a fresh one from the source when there is none
/// or `refresh` is set.
async fn pages(
    state: &AppState,
    remote: &RemoteContent,
    refresh: bool,
) -> Result<Vec<Image>, ApiError> {
    let id = remote.publication_id;
    if !remote.available {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            code: "unavailable",
            message: format!("publication {id}'s chapter is no longer listed by its source"),
        });
    }
    if !refresh {
        let stored = blocking(state, move |c| c.remote_pages(id)).await?;
        if !stored.is_empty() {
            return Ok(stored
                .iter()
                .filter_map(|json| image_from_json(json.as_bytes()))
                .collect());
        }
    }
    let source = library_source(state, remote.library_id).await?;
    let chapter_id = remote.remote_id.clone();
    let pages = run(source, move |s| s.pages(&chapter_id)).await?;
    let keys: Vec<String> = pages.iter().map(image_json).collect();
    blocking(state, move |c| c.set_remote_pages(id, &keys)).await?;
    Ok(pages)
}

/// Reads a series and its chapters from the source and records them.
async fn record_series(
    state: &AppState,
    library_id: i64,
    source: Arc<Source>,
    remote_id: String,
) -> Result<i64, ApiError> {
    let lookup = remote_id.clone();
    let series = run(Arc::clone(&source), move |s| s.series(&lookup)).await?;
    let lookup = remote_id.clone();
    let chapters = run(Arc::clone(&source), move |s| s.chapters(&lookup)).await?;
    let cover = match &series.cover {
        Some(image) => save_cover(state, source, image.clone()).await,
        None => None,
    };
    let record = RemoteSeries {
        sort_title: chek_formats::natural_key(&series.title),
        title: series.title,
        reading_direction: series.reading_direction.map(|d| {
            match d {
                ReadingDirection::Ltr => "ltr",
                ReadingDirection::Rtl => "rtl",
                ReadingDirection::Ttb => "ttb",
            }
            .to_owned()
        }),
        summary: series.summary,
        contributors: series
            .contributors
            .into_iter()
            .map(|c| ContributorRecord {
                name: c.name,
                role: c.role,
            })
            .collect(),
        cover,
        remote_id,
    };
    let chapters: Vec<RemoteChapter> = chapters
        .into_iter()
        .map(|c| {
            let title = match (c.title.trim(), &c.number) {
                ("", Some(number)) => format!("Chapter {number}"),
                ("", None) => c.id.clone(),
                (title, _) => title.to_owned(),
            };
            RemoteChapter {
                sort_title: chek_formats::natural_key(&title),
                sort_number: c.number.as_deref().and_then(chek_formats::leading_number),
                number_label: c.number,
                title,
                remote_id: c.id,
                version: c.version,
                published: c.published,
            }
        })
        .collect();
    blocking(state, move |c| {
        c.record_remote_series(library_id, &record, &chapters)
    })
    .await
}

/// Fetches a series' cover and stores its thumbnail in the covers folder,
/// named by the image's hash like a file's cover. A cover that fails is
/// logged and left out.
async fn save_cover(state: &AppState, source: Arc<Source>, image: Image) -> Option<String> {
    let fetched = match run(source, move |s| s.fetch_image(&image)).await {
        Ok(fetched) => fetched,
        Err(e) => {
            warn!(error = %e.message, "couldn't fetch a series cover");
            return None;
        }
    };
    let covers_dir = state.scan.covers_dir.clone();
    let saved = tokio::task::spawn_blocking(move || {
        let name = blake3::hash(&fetched.bytes).to_hex().to_string();
        let target = covers_dir.join(format!("{name}.jpg"));
        if target.exists() {
            return Ok(name);
        }
        let jpeg = chek_formats::cover_from_image(&fetched.bytes).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&covers_dir).map_err(|e| e.to_string())?;
        // Write then rename, so a reader never sees half a cover.
        let partial = covers_dir.join(format!("{name}.jpg.partial"));
        std::fs::write(&partial, jpeg)
            .and_then(|()| std::fs::rename(&partial, &target))
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(name)
    })
    .await;
    match saved {
        Ok(Ok(name)) => Some(name),
        Ok(Err(e)) => {
            warn!(error = %e, "couldn't save a series cover");
            None
        }
        Err(_) => None,
    }
}

/// The loaded source a library runs on, through the install of the admin
/// who made it.
async fn library_source(state: &AppState, library_id: i64) -> Result<Arc<Source>, ApiError> {
    let library = blocking(state, move |c| c.library(library_id))
        .await?
        .ok_or_else(|| ApiError::library_not_found(library_id))?;
    let config = library
        .source_config
        .as_deref()
        .and_then(extension_config)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "library {library_id} doesn't run on an extension, so series can't be followed into it"
            ))
        })?;
    let extension_id = config.extension_id.clone();
    let (install, repository, allow_any) = blocking(state, move |c| {
        let install = c.extension_install(config.user_id, &extension_id)?;
        let repository = match &install {
            Some(install) => c.extension_repository(install.repository_id)?,
            None => None,
        };
        Ok((install, repository, c.allow_any_repository()?))
    })
    .await?;
    let Some(install) = install else {
        return Err(ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "source_unavailable",
            message: format!(
                "the extension {:?} this library runs on is no longer installed by the admin who made it",
                config.extension_id
            ),
        });
    };
    if repository.is_none_or(|r| r.owner_id.is_some() && !allow_any) {
        return Err(not_usable());
    }
    let extensions = Arc::clone(&state.extensions);
    tokio::task::spawn_blocking(move || extensions.source(&install.sha256, install.user_id))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}
