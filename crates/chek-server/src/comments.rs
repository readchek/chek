//! Readers' comments on a chapter, read through an extension that gives
//! them. Read-only: the routes here only pass on what the source's site
//! shows, and nothing is stored.
//!
//! Comments are off until an admin turns on `extensionComments`, and then
//! only extensions from server-wide repositories give them, so a user's own
//! repository can't add them. Avatars and attached pictures come back as
//! links to the image route, so readers never contact the site.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chek_sources::{Comment, CommentOrder, CommentPage, CommentRun, Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::{ApiError, AppState, blocking};
use crate::auth::CurrentUser;
use crate::browse::{cached, image_url, run, usable_source};

/// Comments change as people post, but a page a few minutes old is fine to
/// read, and each fresh one costs a trip to the site.
const PAGE_TTL: Duration = Duration::from_secs(600);
/// Counts change less than readers notice.
const SUMMARY_TTL: Duration = Duration::from_secs(1800);
const PAGE_MAX_AGE: &str = "private, max-age=600";
const SUMMARY_MAX_AGE: &str = "private, max-age=1800";
/// Answers kept at once, past which the oldest go.
const KEPT: usize = 512;
/// The longest chapter id, comment id or cursor taken from a client.
const MAX_ID_BYTES: usize = 1024;

/// Comment answers read from sources lately, per user's install, since a
/// reader opening the panel twice shouldn't send two trips to the site.
#[derive(Default)]
pub struct CommentPages {
    entries: Mutex<HashMap<String, (Instant, Duration, Value)>>,
}

impl CommentPages {
    fn get(&self, key: &str) -> Option<Value> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .get(key)
            .filter(|(read, ttl, _)| read.elapsed() < *ttl)
            .map(|(_, _, value)| value.clone())
    }

    fn insert(&self, key: String, ttl: Duration, value: Value) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, (read, ttl, _)| read.elapsed() < *ttl);
        if entries.len() >= KEPT
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, (read, _, _))| *read)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(key, (Instant::now(), ttl, value));
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryQuery {
    chapter_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    chapter_id: String,
    /// The source's default when left out.
    order: Option<String>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepliesQuery {
    chapter_id: String,
    comment_id: String,
    cursor: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryDto {
    count: u32,
    url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommentPageDto {
    items: Vec<CommentDto>,
    next: Option<String>,
    total: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommentDto {
    id: String,
    author: AuthorDto,
    published: Option<String>,
    edited: bool,
    body: Vec<RunDto>,
    images: Vec<String>,
    spoiler: bool,
    score: Option<i32>,
    reply_to: Option<String>,
    reply_count: u32,
    url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorDto {
    name: String,
    avatar_url: Option<String>,
    badge: Option<String>,
}

/// A stretch of text in one style. Styles that are off are left out.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunDto {
    text: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    bold: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    italic: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    strike: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    code: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    spoiler: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    quote: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
}

impl RunDto {
    fn new(run: CommentRun) -> Self {
        use chek_sources::RunStyle;
        Self {
            bold: run.style.contains(RunStyle::BOLD),
            italic: run.style.contains(RunStyle::ITALIC),
            strike: run.style.contains(RunStyle::STRIKE),
            code: run.style.contains(RunStyle::CODE),
            spoiler: run.style.contains(RunStyle::SPOILER),
            quote: run.style.contains(RunStyle::QUOTE),
            text: run.text,
            link: run.link,
        }
    }
}

fn comment_dto(id: &str, comment: Comment) -> CommentDto {
    CommentDto {
        id: comment.id,
        author: AuthorDto {
            name: comment.author.name,
            avatar_url: comment.author.avatar.as_ref().map(|i| image_url(id, i)),
            badge: comment.author.badge,
        },
        published: comment.published,
        edited: comment.edited,
        body: comment.body.into_iter().map(RunDto::new).collect(),
        images: comment.images.iter().map(|i| image_url(id, i)).collect(),
        spoiler: comment.spoiler,
        score: comment.score,
        reply_to: comment.reply_to,
        reply_count: comment.reply_count,
        url: comment.url,
    }
}

fn page_dto(id: &str, page: CommentPage) -> CommentPageDto {
    CommentPageDto {
        items: page.items.into_iter().map(|c| comment_dto(id, c)).collect(),
        next: page.next,
        total: page.total,
    }
}

/// How many comments a chapter has and where to read them on the site.
/// 204 when the chapter has no thread.
pub async fn summary(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<SummaryQuery>,
) -> Result<Response, ApiError> {
    check_id("chapterId", &query.chapter_id)?;
    let source = commenting_source(&state, &current, &id).await?;
    let key = cache_key(&source, &current, &id, &["summary", &query.chapter_id]);
    let value = match state.comment_pages.get(&key) {
        Some(value) => value,
        None => {
            let chapter_id = query.chapter_id;
            let summary = run(source, move |s| s.comment_summary(&chapter_id)).await?;
            let value = match summary {
                Some(summary) => serde_json::to_value(SummaryDto {
                    count: summary.count,
                    url: summary.url,
                })
                .map_err(ApiError::internal)?,
                None => Value::Null,
            };
            state.comment_pages.insert(key, SUMMARY_TTL, value.clone());
            value
        }
    };
    if value.is_null() {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    Ok(cached(SUMMARY_MAX_AGE, value).into_response())
}

/// A page of a chapter's top-level comments.
pub async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    check_id("chapterId", &query.chapter_id)?;
    if let Some(cursor) = &query.cursor {
        check_id("cursor", cursor)?;
    }
    let source = commenting_source(&state, &current, &id).await?;
    let orders = source.comment_orders().unwrap_or_default();
    let order = match query.order.as_deref() {
        None => orders.first().copied().ok_or_else(count_only)?,
        Some(name) => orders
            .iter()
            .copied()
            .find(|o| chek_sources::order_name(*o) == name)
            .ok_or_else(|| unknown_order(name, orders))?,
    };
    let cursor = query.cursor.unwrap_or_default();
    let key = cache_key(
        &source,
        &current,
        &id,
        &[
            "list",
            &query.chapter_id,
            chek_sources::order_name(order),
            &cursor,
        ],
    );
    let value = match state.comment_pages.get(&key) {
        Some(value) => value,
        None => {
            let chapter_id = query.chapter_id;
            let page = run(source, move |s| {
                s.comments(
                    &chapter_id,
                    order,
                    Some(cursor.as_str()).filter(|c| !c.is_empty()),
                )
            })
            .await?;
            let value = serde_json::to_value(page_dto(&id, page)).map_err(ApiError::internal)?;
            state.comment_pages.insert(key, PAGE_TTL, value.clone());
            value
        }
    };
    Ok(cached(PAGE_MAX_AGE, value).into_response())
}

/// A page of the replies to one comment.
pub async fn replies(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<RepliesQuery>,
) -> Result<Response, ApiError> {
    check_id("chapterId", &query.chapter_id)?;
    check_id("commentId", &query.comment_id)?;
    if let Some(cursor) = &query.cursor {
        check_id("cursor", cursor)?;
    }
    let source = commenting_source(&state, &current, &id).await?;
    if source
        .comment_orders()
        .is_none_or(<[CommentOrder]>::is_empty)
    {
        return Err(count_only());
    }
    let cursor = query.cursor.unwrap_or_default();
    let key = cache_key(
        &source,
        &current,
        &id,
        &["replies", &query.chapter_id, &query.comment_id, &cursor],
    );
    let value = match state.comment_pages.get(&key) {
        Some(value) => value,
        None => {
            let (chapter_id, comment_id) = (query.chapter_id, query.comment_id);
            let page = run(source, move |s| {
                s.comment_replies(
                    &chapter_id,
                    &comment_id,
                    Some(cursor.as_str()).filter(|c| !c.is_empty()),
                )
            })
            .await?;
            let value = serde_json::to_value(page_dto(&id, page)).map_err(ApiError::internal)?;
            state.comment_pages.insert(key, PAGE_TTL, value.clone());
            value
        }
    };
    Ok(cached(PAGE_MAX_AGE, value).into_response())
}

/// The user's install of `id`, when it may give comments: the admin turned
/// them on, it came from a server-wide repository, and it exports them.
async fn commenting_source(
    state: &AppState,
    current: &CurrentUser,
    id: &str,
) -> Result<Arc<Source>, ApiError> {
    let (install, source) = usable_source(state, current, id).await?;
    let repository_id = install.repository_id;
    let (on, server_wide) = blocking(state, move |c| {
        Ok((
            c.extension_comments()?,
            c.extension_repository(repository_id)?
                .is_some_and(|r| r.owner_id.is_none()),
        ))
    })
    .await?;
    if !on || !server_wide || source.comment_orders().is_none() {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            code: "comments_unavailable",
            message: format!(
                "comments aren't available for {id:?}; an admin turns on extensionComments in /api/v1/settings, and only extensions from server-wide repositories that give comments answer them"
            ),
        });
    }
    Ok(source)
}

/// Sources are loaded per user with that user's cookies, and a new build
/// may read the site differently, so answers are kept per both.
fn cache_key(source: &Source, current: &CurrentUser, id: &str, parts: &[&str]) -> String {
    let mut key = format!("{} {} {}", source.digest(), current.user.id, id);
    for part in parts {
        key.push('\u{1f}');
        key.push_str(part);
    }
    key
}

fn check_id(name: &str, value: &str) -> Result<(), ApiError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(ApiError::bad_request(format!(
            "{name} must be between 1 and {MAX_ID_BYTES} bytes"
        )));
    }
    Ok(())
}

fn count_only() -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        code: "comments_unavailable",
        message:
            "this source gives only a count of comments; use the summary route and link to the site"
                .to_owned(),
    }
}

fn unknown_order(name: &str, orders: &[CommentOrder]) -> ApiError {
    let names: Vec<&str> = orders
        .iter()
        .map(|o| chek_sources::order_name(*o))
        .collect();
    ApiError::bad_request(format!(
        "{name:?} isn't an order this source lists comments in; use one of {names:?}"
    ))
}
