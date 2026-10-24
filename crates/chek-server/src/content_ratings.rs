//! The content ratings each user chose to see. One choice covers every
//! source: it's passed to each extension's search, and results rated
//! outside it are dropped whatever the extension did with it.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use chek_sources::ContentRating;
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, AppState, blocking};
use crate::auth::CurrentUser;

/// Every rating, mildest first.
const ALL: [ContentRating; 4] = [
    ContentRating::Safe,
    ContentRating::Suggestive,
    ContentRating::Erotica,
    ContentRating::Pornographic,
];

/// What a user who never chose sees: nothing sexual beyond fan service.
const DEFAULT: [ContentRating; 2] = [ContentRating::Safe, ContentRating::Suggestive];

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentRatingsDto {
    content_ratings: Vec<String>,
}

impl ContentRatingsDto {
    fn new(ratings: &[ContentRating]) -> Self {
        Self {
            content_ratings: ratings.iter().map(|r| name(*r).to_owned()).collect(),
        }
    }
}

pub async fn get_content_ratings(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<ContentRatingsDto>, ApiError> {
    let ratings = content_ratings(&state, &current).await?;
    Ok(Json(ContentRatingsDto::new(&ratings)))
}

/// Replaces the ratings the user sees. Demo accounts are shared, so one
/// visitor's choice would be the next one's; they keep the default.
pub async fn set_content_ratings(
    State(state): State<AppState>,
    current: CurrentUser,
    body: Result<Json<ContentRatingsDto>, JsonRejection>,
) -> Result<Json<ContentRatingsDto>, ApiError> {
    current.refuse_demo()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let mut chosen = Vec::new();
    for text in &body.content_ratings {
        let rating = parse(text).ok_or_else(|| {
            ApiError::bad_request(format!(
                "{text:?} isn't a content rating; use safe, suggestive, erotica or pornographic"
            ))
        })?;
        chosen.push(rating);
    }
    let ratings: Vec<ContentRating> = ALL.into_iter().filter(|r| chosen.contains(r)).collect();
    if ratings.is_empty() {
        return Err(ApiError::bad_request(
            "contentRatings needs at least one rating, or nothing would show",
        ));
    }
    let stored = join(&ratings);
    let user = current.user.id;
    blocking(&state, move |c| {
        c.set_content_rating_preference(user, &stored)
    })
    .await?;
    Ok(Json(ContentRatingsDto::new(&ratings)))
}

/// The ratings `current` sees, mildest first.
pub(crate) async fn content_ratings(
    state: &AppState,
    current: &CurrentUser,
) -> Result<Vec<ContentRating>, ApiError> {
    if current.demo {
        return Ok(DEFAULT.to_vec());
    }
    let user = current.user.id;
    let stored = blocking(state, move |c| c.content_rating_preference(user)).await?;
    // A stored list that no longer reads, such as one naming a rating since
    // removed, falls back to the default rather than showing everything.
    let ratings: Vec<ContentRating> = stored
        .iter()
        .flat_map(|s| s.split(','))
        .filter_map(parse)
        .collect();
    Ok(if ratings.is_empty() {
        DEFAULT.to_vec()
    } else {
        ratings
    })
}

pub(crate) fn name(rating: ContentRating) -> &'static str {
    match rating {
        ContentRating::Safe => "safe",
        ContentRating::Suggestive => "suggestive",
        ContentRating::Erotica => "erotica",
        ContentRating::Pornographic => "pornographic",
    }
}

fn parse(text: &str) -> Option<ContentRating> {
    ALL.into_iter().find(|r| name(*r) == text)
}

fn join(ratings: &[ContentRating]) -> String {
    ratings
        .iter()
        .map(|r| name(*r))
        .collect::<Vec<_>>()
        .join(",")
}
