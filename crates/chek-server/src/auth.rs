//! Accounts and sign-in.
//!
//! Clients sign in with a name and password and get a random session token,
//! which they send as `Authorization: Bearer <token>`. Only a hash of each
//! token is stored. Passwords are hashed with Argon2id. Sign-in through
//! OpenID Connect providers is in [`crate::oidc`].

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chek_catalog::{CatalogError, NewUser, Role, User, UserChanges, UserUpdate};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::api::{ApiError, AppState, blocking};
use crate::limit::{ClientIp, client_ip};

/// Sessions last this long after their last use.
const SESSION_SECONDS: i64 = 90 * 24 * 60 * 60;
/// A session's expiry is pushed back at most this often, to save writes.
const SESSION_REFRESH_SECONDS: i64 = 60 * 60;
const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_BYTES: usize = 1024;
pub(crate) const MAX_NAME_CHARS: usize = 64;
/// Basic auth checks remembered at once; the set is emptied when full.
const MAX_BASIC_AUTH_CACHE: usize = 10_000;

/// The signed-in user making a request.
pub struct CurrentUser {
    pub user: User,
    /// The device name given at sign-in.
    pub device: Option<String>,
    /// The session's id; `None` for a request signed with Basic auth.
    pub session_id: Option<i64>,
    /// The session token's hash; `None` for a request signed with Basic auth.
    token_hash: Option<String>,
    /// A shared demo account named in `CHEK_DEMO_USERS`. Its password is
    /// public, so it can't change what other visitors rely on, and its
    /// reading places aren't kept.
    pub demo: bool,
}

impl CurrentUser {
    pub fn require_admin(&self) -> Result<(), ApiError> {
        if self.user.role == Role::Admin {
            Ok(())
        } else {
            Err(ApiError {
                status: StatusCode::FORBIDDEN,
                code: "forbidden",
                message: "only an admin can do this".to_owned(),
            })
        }
    }

    /// Refuses changes a shared demo account can't make.
    pub fn refuse_demo(&self) -> Result<(), ApiError> {
        if self.demo {
            Err(ApiError {
                status: StatusCode::FORBIDDEN,
                code: "demo_account",
                message: "this is a shared demo account, which can't be changed".to_owned(),
            })
        } else {
            Ok(())
        }
    }
}

fn is_demo(state: &AppState, user: &User) -> bool {
    state.demo_users.contains(&user.name)
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let credentials = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .map(|(scheme, value)| (scheme, value.trim()))
            .filter(|(_, value)| !value.is_empty());
        let token = match credentials {
            Some((scheme, token)) if scheme.eq_ignore_ascii_case("Bearer") => token,
            Some((scheme, encoded)) if scheme.eq_ignore_ascii_case("Basic") => {
                let ip = client_ip(parts, state);
                return basic_user(state, encoded, ip).await;
            }
            _ => {
                return Err(unauthorized(
                    "sign in and send the session token as `Authorization: Bearer <token>`",
                ));
            }
        };
        let token_hash = hash_token(token);
        let lookup = token_hash.clone();
        let now = now();
        let session = blocking(state, move |c| {
            c.session_user(&lookup, now, SESSION_REFRESH_SECONDS, now + SESSION_SECONDS)
        })
        .await?
        .ok_or_else(|| {
            unauthorized("the session token is invalid or has expired; sign in again")
        })?;
        Ok(Self {
            demo: is_demo(state, &session.user),
            user: session.user,
            device: session.device,
            session_id: Some(session.session_id),
            token_hash: Some(token_hash),
        })
    }
}

/// Signs a request in with HTTP Basic credentials, which OPDS reader apps
/// send instead of session tokens. Checking an Argon2 hash on every page
/// request would be slow, so a successful check is remembered for that exact
/// password and stored hash; changing the password changes the hash and
/// forgets it.
async fn basic_user(
    state: &AppState,
    encoded: &str,
    ip: Option<IpAddr>,
) -> Result<CurrentUser, ApiError> {
    let invalid = || unauthorized("the name or password is wrong");
    let decoded = STANDARD.decode(encoded).map_err(|_| invalid())?;
    let decoded = String::from_utf8(decoded).map_err(|_| invalid())?;
    let (name, password) = decoded.split_once(':').ok_or_else(invalid)?;
    let (name, password) = (name.trim().to_owned(), password.to_owned());
    let found = blocking(state, move |c| c.user_by_name(&name)).await?;
    let Some(found) = found else {
        state.password_limiter.check(ip)?;
        // Same cost as a real check, so timing doesn't reveal which names exist.
        verify_password(password, dummy_hash().to_owned()).await?;
        state.password_limiter.record_failure(ip);
        return Err(invalid());
    };
    let key = *blake3::Hasher::new()
        .update(found.password_hash.as_bytes())
        .update(&[0])
        .update(password.as_bytes())
        .finalize()
        .as_bytes();
    let known = state
        .basic_auth_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&key);
    // A remembered password needs no check, so apps already signed in keep
    // working while someone else at the same address is locked out.
    if !known {
        state.password_limiter.check(ip)?;
        if !verify_password(password, found.password_hash).await? {
            state.password_limiter.record_failure(ip);
            return Err(invalid());
        }
        let mut cache = state
            .basic_auth_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cache.len() >= MAX_BASIC_AUTH_CACHE {
            cache.clear();
        }
        cache.insert(key);
    }
    Ok(CurrentUser {
        demo: is_demo(state, &found.user),
        user: found.user,
        device: None,
        session_id: None,
        token_hash: None,
    })
}

fn unauthorized(message: &str) -> ApiError {
    ApiError {
        status: StatusCode::UNAUTHORIZED,
        code: "unauthorized",
        message: message.to_owned(),
    }
}

/// Logs a one-time setup token when the server has no accounts, so only
/// someone who can read the server's log can create the first admin. A
/// token set in `CHEK_SETUP_TOKEN` is used instead and isn't logged, since
/// whoever set it already knows it.
pub async fn prepare_setup(
    state: &AppState,
    configured: Option<String>,
) -> Result<(), CatalogError> {
    let catalog = state.catalog.clone();
    let users = tokio::task::spawn_blocking(move || catalog.user_count())
        .await
        .expect("counting users does not panic")?;
    if users == 0 {
        let token = match configured {
            Some(token) => {
                warn!(
                    "no accounts yet; create the first admin with POST /api/v1/setup and CHEK_SETUP_TOKEN"
                );
                token
            }
            None => {
                let token = random_token();
                warn!(
                    setup_token = %token,
                    "no accounts yet; create the first admin with POST /api/v1/setup and this setup token"
                );
                token
            }
        };
        *state.setup_token.lock().unwrap_or_else(|e| e.into_inner()) = Some(token);
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    id: i64,
    name: String,
    role: &'static str,
    library_ids: Vec<i64>,
    created_at: String,
    /// Only `/me` says so, since it's about who is asking.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    demo: bool,
}

impl From<User> for UserDto {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            name: u.name,
            role: u.role.as_str(),
            library_ids: u.library_ids,
            created_at: u.created_at,
            demo: false,
        }
    }
}

#[derive(Serialize)]
pub struct SessionDto {
    pub(crate) token: String,
    pub(crate) user: UserDto,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SetupRequest {
    setup_token: String,
    name: String,
    password: String,
}

/// Creates the first admin and signs them in.
pub async fn setup(
    State(state): State<AppState>,
    body: Result<Json<SetupRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<SessionDto>), ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let expected = state
        .setup_token
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(expected) = expected else {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "already_set_up",
            message: "the server already has an admin; sign in instead".to_owned(),
        });
    };
    if !constant_time_eq(body.setup_token.as_bytes(), expected.as_bytes()) {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message:
                "the setup token doesn't match the one in the server's log or CHEK_SETUP_TOKEN"
                    .to_owned(),
        });
    }
    let name = valid_name(&body.name)?;
    let password_hash = hash_password(valid_password(body.password)?).await?;

    let user = blocking(&state, move |c| {
        c.create_user(
            &NewUser {
                name: &name,
                password_hash: &password_hash,
                role: Role::Admin,
                library_ids: &[],
            },
            true,
        )
    })
    .await?
    .ok_or_else(|| ApiError {
        status: StatusCode::CONFLICT,
        code: "already_set_up",
        message: "the server already has an admin; sign in instead".to_owned(),
    })?;
    *state.setup_token.lock().unwrap_or_else(|e| e.into_inner()) = None;
    info!(user = %user.name, "first admin created");
    let token = start_session(&state, user.id, Some("setup")).await?;
    Ok((
        StatusCode::CREATED,
        Json(SessionDto {
            token,
            user: user.into(),
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignInRequest {
    name: String,
    password: String,
    /// A label for the session, such as "Arya's iPad".
    device: Option<String>,
}

pub async fn sign_in(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    body: Result<Json<SignInRequest>, JsonRejection>,
) -> Result<Json<SessionDto>, ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    state.password_limiter.check(ip)?;
    let name = body.name.trim().to_owned();
    let found = blocking(&state, move |c| c.user_by_name(&name)).await?;
    // Verify against a dummy hash when the name is unknown, so the response
    // time doesn't reveal which names exist.
    let (user, hash) = match found {
        Some(found) => (Some(found.user), found.password_hash),
        None => (None, dummy_hash().to_owned()),
    };
    let verified = verify_password(body.password, hash).await?;
    let user = match (user, verified) {
        (Some(user), true) => user,
        _ => {
            state.password_limiter.record_failure(ip);
            return Err(unauthorized("the name or password is wrong"));
        }
    };
    let device = body
        .device
        .map(|d| d.chars().take(MAX_NAME_CHARS).collect::<String>());
    let token = start_session(&state, user.id, device.as_deref()).await?;
    Ok(Json(SessionDto {
        token,
        user: user.into(),
    }))
}

pub async fn sign_out(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<StatusCode, ApiError> {
    // Basic auth has no session to end.
    if let Some(token_hash) = current.token_hash {
        blocking(&state, move |c| c.delete_session(&token_hash)).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(current: CurrentUser) -> Json<UserDto> {
    Json(UserDto {
        demo: current.demo,
        ..current.user.into()
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfoDto {
    id: i64,
    device: Option<String>,
    created_at: String,
    last_used_at: String,
    expires_at: String,
    /// Whether this is the session making the request.
    current: bool,
}

/// The signed-in user's sessions, most recently used first.
pub async fn list_sessions(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<SessionInfoDto>>, ApiError> {
    let user_id = current.user.id;
    let now = now();
    let sessions = blocking(&state, move |c| c.sessions(user_id, now)).await?;
    Ok(Json(
        sessions
            .into_iter()
            // Other visitors' devices are theirs to know.
            .filter(|s| !current.demo || Some(s.id) == current.session_id)
            .map(|s| SessionInfoDto {
                id: s.id,
                device: s.device,
                created_at: s.created_at,
                // Refreshed at most hourly, so this is approximate.
                last_used_at: s.last_used_at,
                expires_at: s.expires_at,
                current: Some(s.id) == current.session_id,
            })
            .collect(),
    ))
}

/// Ends one of the signed-in user's sessions, such as a lost device's.
pub async fn delete_session(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    // Visitors would sign each other out.
    current.refuse_demo()?;
    let user_id = current.user.id;
    if blocking(&state, move |c| c.delete_user_session(user_id, id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("session {id} does not exist")))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    /// Not needed by accounts made by a sign-in provider, which have none.
    current_password: Option<String>,
    new_password: String,
}

/// Changes the signed-in user's password and signs out their other sessions.
/// An account made by a sign-in provider sets its first password this way.
pub async fn change_password(
    State(state): State<AppState>,
    current: CurrentUser,
    body: Result<Json<ChangePasswordRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    current.refuse_demo()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let user_id = current.user.id;
    let hash = blocking(&state, move |c| c.password_hash(user_id))
        .await?
        .ok_or_else(|| unauthorized("the account no longer exists"))?;
    let verified = match body.current_password {
        _ if hash.is_empty() => true,
        Some(password) => verify_password(password, hash).await?,
        None => false,
    };
    if !verified {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: "the current password is wrong".to_owned(),
        });
    }
    let new_hash = hash_password(valid_password(body.new_password)?).await?;
    let keep = current.token_hash;
    blocking(&state, move |c| {
        c.update_user(
            user_id,
            &UserChanges {
                password_hash: Some(&new_hash),
                ..UserChanges::default()
            },
        )?;
        c.delete_user_sessions(user_id, keep.as_deref())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct UserList {
    items: Vec<UserDto>,
}

pub async fn list_users(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<UserList>, ApiError> {
    current.require_admin()?;
    let users = blocking(&state, |c| c.users()).await?;
    Ok(Json(UserList {
        items: users.into_iter().map(Into::into).collect(),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateUserRequest {
    name: String,
    password: String,
    role: RoleDto,
    #[serde(default)]
    library_ids: Vec<i64>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum RoleDto {
    Admin,
    Reader,
}

impl From<RoleDto> for Role {
    fn from(r: RoleDto) -> Self {
        match r {
            RoleDto::Admin => Role::Admin,
            RoleDto::Reader => Role::Reader,
        }
    }
}

pub async fn create_user(
    State(state): State<AppState>,
    current: CurrentUser,
    body: Result<Json<CreateUserRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<UserDto>), ApiError> {
    current.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let name = valid_name(&body.name)?;
    check_libraries(&state, &body.library_ids).await?;
    let password_hash = hash_password(valid_password(body.password)?).await?;
    let role = Role::from(body.role);
    let library_ids = body.library_ids;
    let user = blocking(&state, move |c| {
        c.create_user(
            &NewUser {
                name: &name,
                password_hash: &password_hash,
                role,
                library_ids: &library_ids,
            },
            false,
        )
    })
    .await?
    .ok_or_else(|| ApiError::internal("user was not created"))?;
    Ok((StatusCode::CREATED, Json(user.into())))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateUserRequest {
    password: Option<String>,
    role: Option<RoleDto>,
    library_ids: Option<Vec<i64>>,
}

/// Changes another user's role, library grants or password. A new password
/// signs that user out everywhere.
pub async fn update_user(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    body: Result<Json<UpdateUserRequest>, JsonRejection>,
) -> Result<Json<UserDto>, ApiError> {
    current.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    if let Some(ids) = &body.library_ids {
        check_libraries(&state, ids).await?;
    }
    let password_hash = match body.password {
        Some(p) => Some(hash_password(valid_password(p)?).await?),
        None => None,
    };
    let role = body.role.map(Role::from);
    let library_ids = body.library_ids;
    let keep = (id == current.user.id)
        .then_some(current.token_hash)
        .flatten();
    let update = blocking(&state, move |c| {
        let update = c.update_user(
            id,
            &UserChanges {
                password_hash: password_hash.as_deref(),
                role,
                library_ids: library_ids.as_deref(),
            },
        )?;
        if password_hash.is_some() {
            c.delete_user_sessions(id, keep.as_deref())?;
        }
        Ok(update)
    })
    .await?;
    match update {
        Some(UserUpdate::Updated(user)) => Ok(Json(user.into())),
        Some(UserUpdate::LastAdmin) => Err(last_admin()),
        Some(UserUpdate::Deleted) | None => {
            Err(ApiError::not_found(format!("user {id} does not exist")))
        }
    }
}

pub async fn delete_user(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require_admin()?;
    match blocking(&state, move |c| c.delete_user(id)).await? {
        Some(UserUpdate::Deleted) => Ok(StatusCode::NO_CONTENT),
        Some(UserUpdate::LastAdmin) => Err(last_admin()),
        _ => Err(ApiError::not_found(format!("user {id} does not exist"))),
    }
}

fn last_admin() -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "last_admin",
        message: "the server needs at least one admin; make someone else an admin first".to_owned(),
    }
}

async fn check_libraries(state: &AppState, ids: &[i64]) -> Result<(), ApiError> {
    let wanted = ids.to_vec();
    let found = blocking(state, move |c| c.existing_libraries(&wanted)).await?;
    match ids.iter().find(|id| !found.contains(id)) {
        Some(id) => Err(ApiError::bad_request(format!(
            "library {id} does not exist"
        ))),
        None => Ok(()),
    }
}

fn valid_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request(format!(
            "name must be 1 to {MAX_NAME_CHARS} characters with no control characters"
        )));
    }
    Ok(name.to_owned())
}

fn valid_password(password: String) -> Result<String, ApiError> {
    if password.chars().count() < MIN_PASSWORD_CHARS || password.len() > MAX_PASSWORD_BYTES {
        return Err(ApiError::bad_request(format!(
            "password must be at least {MIN_PASSWORD_CHARS} characters and at most {MAX_PASSWORD_BYTES} bytes"
        )));
    }
    Ok(password)
}

pub(crate) async fn start_session(
    state: &AppState,
    user_id: i64,
    device: Option<&str>,
) -> Result<String, ApiError> {
    let token = random_token();
    let token_hash = hash_token(&token);
    let device = device.map(str::to_owned);
    let now = now();
    blocking(state, move |c| {
        c.delete_expired_sessions(now)?;
        c.create_session(
            &token_hash,
            user_id,
            device.as_deref(),
            now,
            now + SESSION_SECONDS,
        )
    })
    .await?;
    Ok(token)
}

/// Hashing is deliberately slow, so it runs on a blocking thread.
async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)?
}

async fn verify_password(password: String, hash: String) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .verify_password(password.as_bytes(), hash.as_str())
            .is_ok()
    })
    .await
    .map_err(ApiError::internal)
}

fn dummy_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        Argon2::default()
            .hash_password(random_token().as_bytes())
            .map(|h| h.to_string())
            .unwrap_or_default()
    })
}

pub(crate) fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides randomness");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
