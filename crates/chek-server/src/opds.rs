//! An OPDS 2 catalog, so existing reader apps can browse libraries and
//! download publications. Reader apps sign in with HTTP Basic credentials;
//! session tokens work too.
//!
//! Spec: <https://drafts.opds.io/opds-2.0>.

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chek_catalog::{PublicationDetails, PublicationSummary};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::api::{self, ApiError, AppState, blocking, check_publication};
use crate::auth::CurrentUser;

const FEED_TYPE: &str = "application/opds+json";
const PUBLICATION_TYPE: &str = "application/opds-publication+json";
const AUTH_TYPE: &str = "application/opds-authentication+json";
const AUTH_PATH: &str = "/opds/v2/auth";
const PAGE_SIZE: u32 = 50;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/opds/v2/catalog", get(catalog))
        .route("/opds/v2/libraries/{id}", get(library))
        .route("/opds/v2/publications/{id}", get(publication))
        // The same handlers as the API, under this prefix so reader apps
        // send the credentials they used for the catalog.
        .route("/opds/v2/publications/{id}/file", get(api::get_file))
        .route("/opds/v2/publications/{id}/cover", get(api::get_cover))
        .route(AUTH_PATH, get(authentication))
        .layer(middleware::from_fn(challenge))
}

/// Asks for credentials on a 401 in both ways reader apps understand: a
/// Basic challenge, which KOReader follows, and an Authentication for OPDS
/// document, without which Thorium rejects the catalog as unsupported.
async fn challenge(request: Request, next: Next) -> Response {
    let auth_url = auth_document_url(request.headers());
    let response = next.run(request).await;
    if response.status() != StatusCode::UNAUTHORIZED {
        return response;
    }
    let mut response = (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, AUTH_TYPE)],
        Json(auth_document(&auth_url)),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"Chek\", charset=\"UTF-8\""),
    );
    if let Ok(link) = HeaderValue::from_str(&format!(
        "<{auth_url}>; rel=\"http://opds-spec.org/auth/document\"; type=\"{AUTH_TYPE}\""
    )) {
        headers.insert(header::LINK, link);
    }
    response
}

/// The Authentication for OPDS document, served without credentials as the
/// spec requires. Spec: <https://drafts.opds.io/authentication-for-opds-1.0>.
async fn authentication(headers: HeaderMap) -> Response {
    let url = auth_document_url(&headers);
    document(AUTH_TYPE, auth_document(&url))
}

fn auth_document(url: &str) -> Value {
    json!({
        "id": url,
        "title": "Chek",
        "authentication": [{
            "type": "http://opds-spec.org/auth/basic",
            "labels": { "login": "Name", "password": "Password" },
        }],
    })
}

/// The document's absolute URL, which apps use as its id. Behind a proxy
/// that terminates HTTPS, the scheme comes from `X-Forwarded-Proto`; it only
/// names the document, so a client setting it gains nothing.
fn auth_document_url(headers: &HeaderMap) -> String {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    let Some(host) = host else {
        return AUTH_PATH.to_owned();
    };
    let scheme = match headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
    {
        Some("https") => "https",
        _ => "http",
    };
    format!("{scheme}://{host}{AUTH_PATH}")
}

async fn catalog(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Response, ApiError> {
    let libraries = blocking(&state, |c| c.libraries()).await?;
    let navigation: Vec<Value> = libraries
        .into_iter()
        .filter(|l| current.user.can_read(l.id))
        .map(|l| {
            json!({
                "href": format!("/opds/v2/libraries/{}", l.id),
                "title": l.name,
                "type": FEED_TYPE,
                "rel": "subsection",
            })
        })
        .collect();
    Ok(document(
        FEED_TYPE,
        json!({
            "metadata": { "title": "Chek" },
            "links": [
                { "rel": "self", "href": "/opds/v2/catalog", "type": FEED_TYPE },
                { "rel": "start", "href": "/opds/v2/catalog", "type": FEED_TYPE },
            ],
            "navigation": navigation,
        }),
    ))
}

#[derive(Deserialize)]
struct FeedQuery {
    cursor: Option<String>,
}

async fn library(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<FeedQuery>,
) -> Result<Response, ApiError> {
    if !current.user.can_read(id) {
        return Err(ApiError::library_not_found(id));
    }
    let after = match query.cursor.as_deref() {
        None => None,
        Some(c) => Some(
            c.parse::<i64>()
                .map_err(|_| ApiError::bad_request("cursor is not valid"))?,
        ),
    };
    let found = blocking(&state, move |c| {
        let Some(library) = c.library(id)? else {
            return Ok(None);
        };
        let mut entries = Vec::new();
        for summary in c.publications(id, after, PAGE_SIZE)? {
            let details = c.publication_details(summary.id)?.unwrap_or_default();
            entries.push((summary, details));
        }
        Ok(Some((library, entries)))
    })
    .await?;
    let (library, entries) = found.ok_or_else(|| ApiError::library_not_found(id))?;

    let self_href = match after {
        Some(after) => format!("/opds/v2/libraries/{id}?cursor={after}"),
        None => format!("/opds/v2/libraries/{id}"),
    };
    let mut links = vec![
        json!({ "rel": "self", "href": self_href, "type": FEED_TYPE }),
        json!({ "rel": "start", "href": "/opds/v2/catalog", "type": FEED_TYPE }),
        json!({ "rel": "up", "href": "/opds/v2/catalog", "type": FEED_TYPE }),
    ];
    if entries.len() == PAGE_SIZE as usize
        && let Some((last, _)) = entries.last()
    {
        links.push(json!({
            "rel": "next",
            "href": format!("/opds/v2/libraries/{id}?cursor={}", last.id),
            "type": FEED_TYPE,
        }));
    }
    let publications: Vec<Value> = entries
        .into_iter()
        .map(|(summary, details)| publication_entry(summary, details))
        .collect();
    Ok(document(
        FEED_TYPE,
        json!({
            "metadata": { "title": library.name, "itemsPerPage": PAGE_SIZE },
            "links": links,
            "publications": publications,
        }),
    ))
}

async fn publication(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    check_publication(&state, &current, id).await?;
    let found = blocking(&state, move |c| {
        Ok(c.publication(id)?.zip(c.publication_details(id)?))
    })
    .await?;
    let (summary, details) = found.ok_or_else(|| ApiError::publication_not_found(id))?;
    Ok(document(
        PUBLICATION_TYPE,
        publication_entry(summary, details),
    ))
}

/// A publication in the OPDS 2 shape, with Readium Web Publication metadata.
fn publication_entry(summary: PublicationSummary, details: PublicationDetails) -> Value {
    let id = summary.id;
    let mut metadata = Map::new();
    metadata.insert("@type".into(), "http://schema.org/Book".into());
    metadata.insert("title".into(), summary.title.into());
    metadata.insert(
        "belongsTo".into(),
        json!({ "series": [{ "name": summary.series_title }] }),
    );
    if let Some(pages) = summary.page_count {
        metadata.insert("numberOfPages".into(), pages.into());
    }
    for c in details.contributors {
        // Readium names most roles as their own keys; the rest are generic
        // contributors.
        let key = match c.role.as_str() {
            "author" => "author",
            "illustrator" => "illustrator",
            "penciler" => "penciler",
            "inker" => "inker",
            "colorist" => "colorist",
            "letterer" => "letterer",
            "coverArtist" => "artist",
            "editor" => "editor",
            "translator" => "translator",
            _ => "contributor",
        };
        if let Value::Array(names) = metadata
            .entry(key)
            .or_insert_with(|| Value::Array(Vec::new()))
        {
            names.push(json!({ "name": c.name }));
        }
    }
    for (key, value) in [
        ("description", details.summary),
        ("publisher", details.publisher),
        ("language", details.language),
        ("published", details.published),
        ("readingProgression", details.reading_direction),
    ] {
        if let Some(value) = value {
            metadata.insert(key.into(), value.into());
        }
    }

    let mut links = vec![json!({
        "rel": "self",
        "href": format!("/opds/v2/publications/{id}"),
        "type": PUBLICATION_TYPE,
    })];
    if summary.available {
        links.push(json!({
            "rel": "http://opds-spec.org/acquisition",
            "href": format!("/opds/v2/publications/{id}/file"),
            "type": api::file_media_type(&summary.format),
        }));
    }
    let images: Vec<Value> = if summary.has_cover {
        vec![json!({
            "href": format!("/opds/v2/publications/{id}/cover"),
            "type": "image/jpeg",
        })]
    } else {
        Vec::new()
    };
    json!({ "metadata": metadata, "links": links, "images": images })
}

fn document(media_type: &'static str, body: Value) -> Response {
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static(media_type))],
        body.to_string(),
    )
        .into_response()
}
