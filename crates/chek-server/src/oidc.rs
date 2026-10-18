//! Sign-in through OpenID Connect providers, such as Authentik, Authelia,
//! Keycloak or Google.
//!
//! An app asks Chek for the provider's sign-in page, sending a PKCE code
//! challenge of its own and where to return. The person signs in at the
//! provider, which sends them to Chek's callback. Chek trades the provider's
//! code for the person's identity, then sends the browser on to the app with
//! a one-time code of Chek's. The app trades that code and its PKCE verifier
//! for a session. The session token never appears in a URL, and a code that
//! lands in the wrong app is useless without the verifier.
//!
//! Linking works the same way, started by a signed-in user. The link is made
//! only when the code is traded, so someone can't link their own account to a
//! victim's provider account by sending the victim a sign-in link.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chek_catalog::{LinkOutcome, Role, UnlinkOutcome};
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;
use tracing::{info, warn};

use crate::api::{ApiError, AppState, blocking};
use crate::auth::{self, CurrentUser, UserDto};

/// How long a person has to sign in at the provider.
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);
/// How long an app has to trade Chek's code for a session.
const GRANT_TTL: Duration = Duration::from_secs(5 * 60);
/// Sign-ins in progress at once; more are refused rather than kept.
const MAX_IN_PROGRESS: usize = 10_000;
/// The largest provider response read.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// Provider clocks may run a little ahead or behind.
const CLOCK_SKEW_SECONDS: i64 = 5 * 60;
const MAX_NAME_CHARS: usize = 64;

/// One provider, from `CHEK_OIDC_<ID>_*`.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Lowercase letters, digits and hyphens; names the provider in the API.
    pub id: String,
    /// Shown on the sign-in button.
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    /// `None` for a public client.
    pub client_secret: Option<String>,
    pub scopes: String,
    /// Whether someone with no linked account gets a new one.
    pub create_users: bool,
    /// Accounts made for members of these groups are admins.
    pub admin_groups: Vec<String>,
    /// The claim that lists a person's groups.
    pub groups_claim: String,
}

pub struct Oidc {
    /// Where providers send people back to; `None` when no provider is set.
    callback_url: Option<Url>,
    /// Where apps may ask to be sent after sign-in, as prefixes.
    client_redirects: Vec<String>,
    providers: Vec<Provider>,
    pending: Expiring<Pending>,
    grants: Expiring<Grant>,
}

struct Provider {
    config: ProviderConfig,
    /// Fetched on first use and kept, so a provider that's down at startup
    /// doesn't stop the server.
    discovery: OnceCell<Discovery>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    userinfo_endpoint: Option<String>,
}

/// A sign-in waiting for the provider to send the person back.
struct Pending {
    provider: usize,
    nonce: String,
    verifier: String,
    client_redirect: Url,
    client_challenge: String,
    link_user: Option<i64>,
}

/// A finished sign-in waiting for the app to trade its code.
struct Grant {
    provider: usize,
    subject: String,
    name: String,
    groups: Vec<String>,
    link_user: Option<i64>,
    client_challenge: String,
}

impl Oidc {
    pub fn new(
        public_url: Option<Url>,
        providers: Vec<ProviderConfig>,
        client_redirects: Vec<String>,
    ) -> Self {
        let public_url = public_url.map(|mut url| {
            if !url.path().ends_with('/') {
                let path = format!("{}/", url.path());
                url.set_path(&path);
            }
            url
        });
        let callback_url = public_url
            .as_ref()
            .filter(|_| !providers.is_empty())
            .and_then(|url| url.join("api/v1/auth/callback").ok());
        // The server's own web pages may always be returned to.
        let client_redirects = public_url
            .iter()
            .map(|url| url.to_string())
            .chain(client_redirects)
            .collect();
        Self {
            callback_url,
            client_redirects,
            providers: providers
                .into_iter()
                .map(|config| Provider {
                    config,
                    discovery: OnceCell::new(),
                })
                .collect(),
            pending: Expiring::default(),
            grants: Expiring::default(),
        }
    }

    pub fn callback_url(&self) -> Option<&Url> {
        self.callback_url.as_ref()
    }

    fn provider(&self, id: &str) -> Result<usize, ApiError> {
        self.providers
            .iter()
            .position(|p| p.config.id == id)
            .ok_or_else(|| ApiError::not_found(format!("sign-in provider {id:?} isn't set up")))
    }

    fn client_redirect(&self, uri: &str) -> Result<Url, ApiError> {
        let url = Url::parse(uri)
            .map_err(|e| ApiError::bad_request(format!("redirectUri isn't a URL: {e}")))?;
        if url.fragment().is_some() {
            return Err(ApiError::bad_request("redirectUri can't have a fragment"));
        }
        if !self
            .client_redirects
            .iter()
            .any(|p| redirect_allowed(p, &url))
        {
            return Err(ApiError::bad_request(format!(
                "redirectUri must start with one of {:?}; the server's admin can allow more in CHEK_OIDC_REDIRECTS",
                self.client_redirects
            )));
        }
        Ok(url)
    }
}

/// Whether `url` falls under an allowed prefix. Web prefixes are compared by
/// origin and path, so `https://chek.example` doesn't allow
/// `https://chek.example.evil.test`.
fn redirect_allowed(prefix: &str, url: &Url) -> bool {
    match Url::parse(prefix) {
        Ok(p) if matches!(p.scheme(), "http" | "https") => {
            p.origin() == url.origin() && url.path().starts_with(p.path())
        }
        _ => url.as_str().starts_with(prefix),
    }
}

#[derive(Serialize)]
pub struct ProviderList {
    items: Vec<ProviderDto>,
}

#[derive(Serialize)]
pub struct ProviderDto {
    id: String,
    name: String,
}

/// The providers people can sign in with, for drawing sign-in buttons.
pub async fn list_providers(State(state): State<AppState>) -> Json<ProviderList> {
    Json(ProviderList {
        items: state
            .oidc
            .providers
            .iter()
            .map(|p| ProviderDto {
                id: p.config.id.clone(),
                name: p.config.name.clone(),
            })
            .collect(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AuthorizeRequest {
    redirect_uri: String,
    code_challenge: String,
}

#[derive(Serialize)]
pub struct AuthorizeDto {
    url: String,
}

/// Starts signing in: returns the provider's page to open in a browser.
pub async fn authorize(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<AuthorizeRequest>, JsonRejection>,
) -> Result<Json<AuthorizeDto>, ApiError> {
    start(&state, &id, body, None).await
}

/// Starts linking a provider account to the signed-in user.
pub async fn authorize_link(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    body: Result<Json<AuthorizeRequest>, JsonRejection>,
) -> Result<Json<AuthorizeDto>, ApiError> {
    start(&state, &id, body, Some(current.user.id)).await
}

async fn start(
    state: &AppState,
    id: &str,
    body: Result<Json<AuthorizeRequest>, JsonRejection>,
    link_user: Option<i64>,
) -> Result<Json<AuthorizeDto>, ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let oidc = &state.oidc;
    let index = oidc.provider(id)?;
    let client_redirect = oidc.client_redirect(&body.redirect_uri)?;
    if !is_s256_challenge(&body.code_challenge) {
        return Err(ApiError::bad_request(
            "codeChallenge must be the base64url SHA-256 of a PKCE code verifier, without padding",
        ));
    }
    let callback = oidc
        .callback_url
        .as_ref()
        .ok_or_else(|| ApiError::internal("a provider is set up without CHEK_PUBLIC_URL"))?;
    let provider = &oidc.providers[index];
    let discovery = discover(state, provider).await.map_err(|e| ApiError {
        status: StatusCode::BAD_GATEWAY,
        code: "provider_unavailable",
        message: format!("couldn't reach {}: {e}", provider.config.name),
    })?;

    let nonce = auth::random_token();
    let verifier = auth::random_token();
    let challenge = s256(&verifier);
    let key = oidc.pending.insert(
        PENDING_TTL,
        Pending {
            provider: index,
            nonce: nonce.clone(),
            verifier,
            client_redirect,
            client_challenge: body.code_challenge,
            link_user,
        },
    )?;
    let mut url = Url::parse(&discovery.authorization_endpoint)
        .map_err(|e| ApiError::internal(format!("provider's authorization endpoint: {e}")))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &provider.config.client_id)
        .append_pair("redirect_uri", callback.as_str())
        .append_pair("scope", &provider.config.scopes)
        .append_pair("state", &key)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(Json(AuthorizeDto { url: url.into() }))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Where the provider sends the person back. Sends the browser on to the
/// app, with a one-time code or an error.
pub async fn callback(
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    let pending = query
        .state
        .as_deref()
        .and_then(|key| state.oidc.pending.take(key))
        .ok_or_else(|| {
            ApiError::bad_request("this sign-in has expired or was already used; start again")
        })?;
    let mut target = pending.client_redirect.clone();
    if let Some(error) = query.error {
        let description = query.error_description.unwrap_or_default();
        add_error(&mut target, &error, &description);
        return Ok(see_other(&target));
    }
    let Some(code) = query.code else {
        add_error(&mut target, "invalid_request", "the provider sent no code");
        return Ok(see_other(&target));
    };
    let provider = &state.oidc.providers[pending.provider];
    match identify(&state, provider, &pending, &code).await {
        Ok(identity) => {
            let grant = state.oidc.grants.insert(
                GRANT_TTL,
                Grant {
                    provider: pending.provider,
                    subject: identity.subject,
                    name: identity.name,
                    groups: identity.groups,
                    link_user: pending.link_user,
                    client_challenge: pending.client_challenge,
                },
            );
            match grant {
                Ok(grant) => {
                    target.query_pairs_mut().append_pair("code", &grant);
                }
                Err(e) => add_error(&mut target, "temporarily_unavailable", &e.message),
            }
        }
        Err(e) => {
            warn!(provider = %provider.config.id, error = %e, "provider sign-in failed");
            add_error(
                &mut target,
                "server_error",
                &format!("couldn't sign in with {}: {e}", provider.config.name),
            );
        }
    }
    Ok(see_other(&target))
}

fn add_error(url: &mut Url, error: &str, description: &str) {
    let mut pairs = url.query_pairs_mut();
    pairs.append_pair("error", error);
    if !description.is_empty() {
        pairs.append_pair("error_description", description);
    }
}

fn see_other(url: &Url) -> Response {
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, url.as_str()),
            (header::CACHE_CONTROL, "no-store"),
        ],
    )
        .into_response()
}

struct ProviderIdentity {
    subject: String,
    name: String,
    groups: Vec<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
    access_token: Option<String>,
}

/// Trades the provider's code for the person's identity.
async fn identify(
    state: &AppState,
    provider: &Provider,
    pending: &Pending,
    code: &str,
) -> Result<ProviderIdentity, String> {
    let discovery = discover(state, provider).await?;
    let callback = state
        .oidc
        .callback_url
        .as_ref()
        .ok_or("CHEK_PUBLIC_URL isn't set")?;
    let config = &provider.config;
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", callback.as_str()),
        ("code_verifier", pending.verifier.as_str()),
    ];
    let mut request = state
        .scan
        .http
        .post(&discovery.token_endpoint)
        .header(header::ACCEPT.as_str(), "application/json");
    match &config.client_secret {
        // client_secret_basic is the method every provider must support.
        Some(secret) => {
            request = request.basic_auth(form_encode(&config.client_id), Some(form_encode(secret)));
        }
        None => form.push(("client_id", config.client_id.as_str())),
    }
    let body = serde_urlencoded::to_string(&form).map_err(|e| e.to_string())?;
    let response = request
        .header(
            header::CONTENT_TYPE.as_str(),
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|e| format!("token request failed: {}", e.without_url()))?;
    let tokens: TokenResponse = read_json(response, "token").await?;

    let mut claims = id_token_claims(&tokens.id_token)?;
    check_id_token(&claims, discovery, config, &pending.nonce)?;
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("the ID token has no subject")?
        .to_owned();
    // Some providers, such as Authelia, put profile and group claims only in
    // the userinfo response.
    if let (Some(endpoint), Some(access_token)) =
        (&discovery.userinfo_endpoint, &tokens.access_token)
    {
        match userinfo(state, endpoint, access_token).await {
            Ok(info) if info.get("sub").and_then(Value::as_str) == Some(subject.as_str()) => {
                for (key, value) in info {
                    claims.entry(key).or_insert(value);
                }
            }
            Ok(_) => warn!(provider = %config.id, "ignoring userinfo for a different subject"),
            Err(e) => warn!(provider = %config.id, error = %e, "couldn't read userinfo"),
        }
    }
    let name = ["preferred_username", "name", "email"]
        .iter()
        .filter_map(|key| claims.get(*key).and_then(Value::as_str))
        .map(clean_name)
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "reader".to_owned());
    let groups = match claims.get(&config.groups_claim) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(group)) => vec![group.clone()],
        _ => Vec::new(),
    };
    Ok(ProviderIdentity {
        subject,
        name,
        groups,
    })
}

async fn discover<'a>(state: &AppState, provider: &'a Provider) -> Result<&'a Discovery, String> {
    provider
        .discovery
        .get_or_try_init(|| async {
            let issuer = provider.config.issuer.trim_end_matches('/');
            let response = state
                .scan
                .http
                .get(format!("{issuer}/.well-known/openid-configuration"))
                .send()
                .await
                .map_err(|e| format!("discovery failed: {}", e.without_url()))?;
            let discovery: Discovery = read_json(response, "discovery").await?;
            // OpenID Connect Discovery 1.0, section 4.3.
            if discovery.issuer.trim_end_matches('/') != issuer {
                return Err(format!(
                    "the provider calls itself {:?}, not the configured issuer {:?}",
                    discovery.issuer, provider.config.issuer
                ));
            }
            Ok(discovery)
        })
        .await
}

async fn userinfo(
    state: &AppState,
    endpoint: &str,
    access_token: &str,
) -> Result<Map<String, Value>, String> {
    let response = state
        .scan
        .http
        .get(endpoint)
        .bearer_auth(access_token)
        .header(header::ACCEPT.as_str(), "application/json")
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    read_json(response, "userinfo").await
}

/// Reads a JSON response, failing on an error status or a body over
/// [`MAX_RESPONSE_BYTES`].
async fn read_json<T: DeserializeOwned>(
    mut response: reqwest::Response,
    what: &str,
) -> Result<T, String> {
    let status = response.status();
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("{what} response failed: {}", e.without_url()))?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(format!("the {what} response is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let detail = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| {
                let error = v.get("error")?.as_str()?.to_owned();
                Some(match v.get("error_description").and_then(Value::as_str) {
                    Some(d) => format!("{error}: {d}"),
                    None => error,
                })
            })
            .unwrap_or_default();
        return Err(format!("the {what} request failed with {status} {detail}")
            .trim_end()
            .to_owned());
    }
    serde_json::from_slice(&body).map_err(|e| format!("the {what} response isn't valid: {e}"))
}

/// The ID token's claims. Its signature isn't checked: the token came
/// straight from the provider's token endpoint over TLS, which OpenID Connect
/// Core 1.0 section 3.1.3.7 allows in place of the signature.
fn id_token_claims(token: &str) -> Result<Map<String, Value>, String> {
    let payload = token.split('.').nth(1).ok_or("the ID token isn't a JWT")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| "the ID token isn't a JWT")?;
    serde_json::from_slice(&bytes).map_err(|_| "the ID token's claims aren't JSON".to_owned())
}

fn check_id_token(
    claims: &Map<String, Value>,
    discovery: &Discovery,
    config: &ProviderConfig,
    nonce: &str,
) -> Result<(), String> {
    if claims.get("iss").and_then(Value::as_str) != Some(discovery.issuer.as_str()) {
        return Err("the ID token is from another issuer".to_owned());
    }
    let audiences: Vec<&str> = match claims.get("aud") {
        Some(Value::String(aud)) => vec![aud],
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !audiences.contains(&config.client_id.as_str()) {
        return Err("the ID token is for another client".to_owned());
    }
    if audiences.len() > 1
        && claims.get("azp").and_then(Value::as_str) != Some(config.client_id.as_str())
    {
        return Err("the ID token was issued to another client".to_owned());
    }
    let expires = claims.get("exp").and_then(Value::as_i64).unwrap_or(0);
    if expires + CLOCK_SKEW_SECONDS < auth::now() {
        return Err("the ID token has expired; check the server's clock".to_owned());
    }
    match claims.get("nonce").and_then(Value::as_str) {
        Some(n) if auth::constant_time_eq(n.as_bytes(), nonce.as_bytes()) => Ok(()),
        _ => Err("the ID token's nonce doesn't match this sign-in".to_owned()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ExchangeRequest {
    code: String,
    code_verifier: String,
    /// A label for the session, such as "Arya's iPad".
    device: Option<String>,
}

#[derive(Serialize)]
pub struct ExchangeDto {
    /// Absent when the code finished linking, which starts no session.
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    user: UserDto,
}

/// Trades Chek's one-time code for a session, or finishes a link.
pub async fn exchange(
    State(state): State<AppState>,
    body: Result<Json<ExchangeRequest>, JsonRejection>,
) -> Result<Json<ExchangeDto>, ApiError> {
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    // Taken before checking the verifier, so each code gets one try.
    let grant = state.oidc.grants.take(&body.code).ok_or_else(|| {
        ApiError::bad_request("the code has expired or was already used; sign in again")
    })?;
    if !auth::constant_time_eq(
        s256(&body.code_verifier).as_bytes(),
        grant.client_challenge.as_bytes(),
    ) {
        return Err(ApiError::bad_request(
            "codeVerifier doesn't match the codeChallenge this sign-in started with",
        ));
    }
    let config = state.oidc.providers[grant.provider].config.clone();
    let provider = config.id.clone();
    let subject = grant.subject.clone();

    if let Some(user_id) = grant.link_user {
        let outcome = blocking(&state, move |c| {
            c.link_identity(user_id, &provider, &subject)
        })
        .await?;
        match outcome {
            LinkOutcome::Linked => {}
            LinkOutcome::TakenByOther => {
                return Err(identity_taken(format!(
                    "this {} account is already linked to another Chek account",
                    config.name
                )));
            }
            LinkOutcome::ProviderLinked => {
                return Err(identity_taken(format!(
                    "your Chek account is already linked to another {} account; unlink it first",
                    config.name
                )));
            }
        }
        let user = blocking(&state, move |c| c.user(user_id))
            .await?
            .ok_or_else(|| ApiError::not_found("the account no longer exists".to_owned()))?;
        info!(user = %user.name, provider = %config.id, "linked sign-in provider");
        return Ok(Json(ExchangeDto {
            token: None,
            user: user.into(),
        }));
    }

    let found = blocking(&state, move |c| c.identity_user(&provider, &subject)).await?;
    let user = match found {
        Some(user) => user,
        None if config.create_users => {
            let role = if grant.groups.iter().any(|g| config.admin_groups.contains(g)) {
                Role::Admin
            } else {
                Role::Reader
            };
            let provider = config.id.clone();
            let name = grant.name.clone();
            let user = blocking(&state, move |c| {
                c.create_identity_user(&name, MAX_NAME_CHARS, role, &provider, &grant.subject)
            })
            .await?;
            info!(user = %user.name, provider = %config.id, role = role.as_str(), "created account for provider sign-in");
            user
        }
        None => {
            return Err(ApiError {
                status: StatusCode::FORBIDDEN,
                code: "not_linked",
                message: format!(
                    "no Chek account is linked to this {} account; sign in with a password and link it, or ask an admin",
                    config.name
                ),
            });
        }
    };
    let device = body
        .device
        .map(|d| d.chars().take(MAX_NAME_CHARS).collect::<String>());
    let token = auth::start_session(&state, user.id, device.as_deref()).await?;
    Ok(Json(ExchangeDto {
        token: Some(token),
        user: user.into(),
    }))
}

fn identity_taken(message: String) -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "identity_taken",
        message,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDto {
    provider: String,
    name: String,
    created_at: String,
}

/// The providers linked to the signed-in user.
pub async fn list_identities(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<IdentityDto>>, ApiError> {
    let user_id = current.user.id;
    let identities = blocking(&state, move |c| c.identities(user_id)).await?;
    Ok(Json(
        identities
            .into_iter()
            .map(|i| IdentityDto {
                // A provider removed from the settings keeps its id as a name.
                name: state
                    .oidc
                    .providers
                    .iter()
                    .find(|p| p.config.id == i.provider)
                    .map_or_else(|| i.provider.clone(), |p| p.config.name.clone()),
                provider: i.provider,
                created_at: i.created_at,
            })
            .collect(),
    ))
}

pub async fn unlink(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(provider): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user_id = current.user.id;
    let id = provider.clone();
    match blocking(&state, move |c| c.unlink_identity(user_id, &id)).await? {
        UnlinkOutcome::Unlinked => Ok(StatusCode::NO_CONTENT),
        UnlinkOutcome::NotLinked => Err(ApiError::not_found(format!(
            "no {provider:?} account is linked to yours"
        ))),
        UnlinkOutcome::LastSignIn => Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "last_sign_in",
            message: "this is the only way you can sign in; set a password first".to_owned(),
        }),
    }
}

/// Values kept for a short while under a random key, taken at most once.
struct Expiring<T> {
    items: Mutex<HashMap<String, (Instant, T)>>,
}

impl<T> Default for Expiring<T> {
    fn default() -> Self {
        Self {
            items: Mutex::default(),
        }
    }
}

impl<T> Expiring<T> {
    fn insert(&self, ttl: Duration, value: T) -> Result<String, ApiError> {
        let now = Instant::now();
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        if items.len() >= MAX_IN_PROGRESS {
            items.retain(|_, (expires, _)| *expires > now);
            if items.len() >= MAX_IN_PROGRESS {
                return Err(ApiError {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    code: "too_many_sign_ins",
                    message: "too many sign-ins are in progress; try again in a few minutes"
                        .to_owned(),
                });
            }
        }
        let key = auth::random_token();
        items.insert(key.clone(), (now + ttl, value));
        Ok(key)
    }

    fn take(&self, key: &str) -> Option<T> {
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        let (expires, value) = items.remove(key)?;
        (expires > Instant::now()).then_some(value)
    }
}

fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn is_s256_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Encodes a client id or secret for HTTP Basic auth, as OAuth 2.0
/// (RFC 6749 section 2.3.1) requires.
fn form_encode(value: &str) -> String {
    serde_urlencoded::to_string([("", value)])
        .map(|s| s[1..].to_owned())
        .unwrap_or_default()
}

/// A provider's name for someone, made into a Chek user name.
fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}
