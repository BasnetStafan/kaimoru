//! SNS login service.
//! Owns provider availability (fail-closed, the KOMOJU
//! settings shape), the authorization URL and PKCE values,
//! the server-side code exchange, ID-token verification,
//! and the identity decisions behind the callback and the
//! MyPage link screens (which account a verified identity
//! signs in, links to, or registers).
//! Client secrets never leave this layer and are never
//! logged; a failure is logged with its provider and stage
//! only - never a code, a token, or a secret.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::models::User;
use crate::models::user_identity::UserIdentity;
use crate::repositories::{
    pending_registration_repository, user_identity_repository, user_repository,
};
use crate::services::sns_providers::SnsProvider;
use crate::session;

/// How long a cached JWKS document is trusted before it is
/// refetched; an unknown key id refetches immediately, so
/// provider key rotation is covered either way.
const JWKS_TTL: Duration = Duration::from_secs(3600);

/// One in-flight authorization round trip, parked in the
/// session between the start and callback routes. Single-use:
/// the callback removes it before anything else happens.
#[derive(Clone, Serialize, Deserialize)]
pub struct SnsFlow {
    /// Provider slug the flow was started for.
    pub provider: String,
    /// The `state` value: compared in constant time, once.
    pub state: String,
    /// The `nonce` the ID token must carry.
    pub nonce: String,
    /// The PKCE verifier whose challenge was sent.
    pub code_verifier: String,
    /// Validated post-login destination, when any.
    pub next: Option<String>,
    /// The member who started a link flow; `None` is a
    /// sign-in / registration flow.
    pub link_user_id: Option<i64>,
    /// Unix seconds when the flow started (10-minute expiry).
    pub started_at: i64,
}

/// The verified provider identity awaiting registration
/// completion, parked in the session at the callback.
#[derive(Clone, Serialize, Deserialize)]
pub struct SnsPendingProfile {
    /// Provider slug.
    pub provider: String,
    /// The provider's stable user id (the `sub` claim).
    pub provider_user_id: String,
    /// The provider-reported email, when it reported one.
    pub provider_email: Option<String>,
    /// The provider-reported display name, when any.
    pub display_name: Option<String>,
    /// Whether the provider itself verified the email
    /// (registration may proceed on it directly) or the
    /// address still needs this store's own treatment: code
    /// verification first, link-required if it is taken.
    pub email_trusted: bool,
}

/// A link awaiting the account owner's password sign-in:
/// parked when a provider email matched an existing account,
/// confirmed on the SNS settings page afterwards.
#[derive(Clone, Serialize, Deserialize)]
pub struct SnsLinkPending {
    /// Provider slug.
    pub provider: String,
    /// The provider's stable user id (the `sub` claim).
    pub provider_user_id: String,
    /// The email that matched the existing account.
    pub provider_email: String,
    /// Unix seconds when the marker was parked (30-minute
    /// expiry).
    pub parked_at: i64,
}

/// The claims of one verified ID token: all the flow layer
/// may act on. Nothing else from the token is trusted or
/// kept.
pub struct SnsClaims {
    /// The provider's stable user id.
    pub sub: String,
    /// The provider-reported email, normalised; `None` when
    /// the provider reported none.
    pub email: Option<String>,
    /// Whether the provider verified that email. Always
    /// `false` when there is no email.
    pub email_verified: bool,
    /// The provider-reported display name, when any.
    pub name: Option<String>,
}

/// The stage an SNS sign-in attempt failed at. Logged with
/// the provider; never shown to the visitor in detail.
#[derive(Debug)]
pub enum SnsFailure {
    /// The code exchange with the provider failed.
    Exchange,
    /// The ID token failed verification.
    Verify,
}

impl std::fmt::Display for SnsFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exchange => write!(f, "token exchange failed"),
            Self::Verify => write!(f, "id token verification failed"),
        }
    }
}

impl std::error::Error for SnsFailure {}

/// Availability resolved once per process, like the KOMOJU
/// settings: a half-configured provider hides instead of
/// failing at click time.
struct SnsSettings {
    available: Vec<SnsProvider>,
}

fn settings() -> &'static SnsSettings {
    static SETTINGS: OnceLock<SnsSettings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let mut available = Vec::new();
        if CONFIG.sns_login_enabled {
            if CONFIG.public_base_url().is_none() {
                tracing::error!(
                    "sns login enabled but PUBLIC_BASE_URL is unset; SNS sign-in stays hidden"
                );
            } else {
                for provider in SnsProvider::ALL {
                    if provider.credentials().is_some() {
                        available.push(provider);
                    } else {
                        tracing::warn!(
                            provider = provider.slug(),
                            "sns provider credentials missing; provider stays hidden"
                        );
                    }
                }
            }
        }
        SnsSettings { available }
    })
}

/// The providers currently offered, in display order.
#[must_use]
pub fn available_providers() -> Vec<SnsProvider> {
    settings().available.clone()
}

/// Whether one provider is currently offered.
#[must_use]
pub fn is_available(provider: SnsProvider) -> bool {
    settings().available.contains(&provider)
}

/// The callback URL this provider must redirect to, built
/// from the configured public base URL.
#[must_use]
pub fn redirect_uri(provider: SnsProvider) -> Option<String> {
    CONFIG.public_base_url().map(|base| {
        format!(
            "{}/auth/{}/callback",
            base.trim_end_matches('/'),
            provider.slug()
        )
    })
}

/// The anti-forgery values for one authorization round trip.
pub struct FlowSecrets {
    /// The `state` value.
    pub state: String,
    /// The `nonce` value.
    pub nonce: String,
    /// The PKCE verifier (its S256 challenge is sent).
    pub code_verifier: String,
}

/// Generates fresh flow secrets from the CSPRNG.
#[must_use]
pub fn new_flow_secrets() -> FlowSecrets {
    FlowSecrets {
        state: random_token(),
        nonce: random_token(),
        code_verifier: random_token(),
    }
}

/// Builds the provider authorization URL for one flow:
/// authorization code flow with PKCE (S256) for all three
/// providers.
#[must_use]
pub fn authorize_url(provider: SnsProvider, secrets: &FlowSecrets, redirect_uri: &str) -> String {
    let (client_id, _) = provider.credentials().unwrap_or_default();
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&state={}&scope={}&nonce={}&code_challenge={}&code_challenge_method=S256",
        provider.endpoints().authorize,
        url_encode(&client_id),
        url_encode(redirect_uri),
        secrets.state,
        url_encode(provider.scope()),
        secrets.nonce,
        pkce_challenge(&secrets.code_verifier),
    )
}

/// Exchanges the authorization code at the provider's token
/// endpoint (the client secret travels server-side only) and
/// verifies the returned ID token.
///
/// # Errors
///
/// Returns [`SnsFailure`] naming the failed stage.
pub async fn exchange_and_verify(
    provider: SnsProvider,
    code: &str,
    code_verifier: &str,
    nonce: &str,
    redirect_uri: &str,
) -> Result<SnsClaims, SnsFailure> {
    let (client_id, client_secret) = provider.credentials().ok_or(SnsFailure::Exchange)?;
    let client = reqwest::Client::new();
    let response = client
        .post(provider.endpoints().token)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|_| SnsFailure::Exchange)?;
    if !response.status().is_success() {
        return Err(SnsFailure::Exchange);
    }
    let body: TokenResponse = response.json().await.map_err(|_| SnsFailure::Exchange)?;
    let id_token = body.id_token.ok_or(SnsFailure::Exchange)?;
    verify_id_token(provider, &id_token, &client_id, nonce).await
}

/// The token endpoint's JSON answer; only the ID token is
/// used - no access token is kept or logged.
#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
}

/// Verifies one ID token against the provider's JWKS:
/// signature, issuer, audience (this provider's client id),
/// expiry, and the flow nonce.
async fn verify_id_token(
    provider: SnsProvider,
    token: &str,
    client_id: &str,
    expected_nonce: &str,
) -> Result<SnsClaims, SnsFailure> {
    let header = jsonwebtoken::decode_header(token).map_err(|_| SnsFailure::Verify)?;
    let kid = header.kid.ok_or(SnsFailure::Verify)?;
    let jwk = fetch_jwk(provider, &kid).await?;
    let key = jsonwebtoken::DecodingKey::from_jwk(&jwk).map_err(|_| SnsFailure::Verify)?;
    // The algorithm is pinned, never taken from the token
    // LINE, Google and Yahoo all
    // sign with RS256, and letting the header choose is
    // the classic JOSE algorithm-confusion footgun.
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&[client_id]);
    validation.set_issuer(&[provider.issuer()]);
    validation.leeway = 60;
    let data = jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)
        .map_err(|_| SnsFailure::Verify)?;
    let claims = data.claims;

    let nonce = claims
        .get("nonce")
        .and_then(serde_json::Value::as_str)
        .ok_or(SnsFailure::Verify)?;
    if !session::constant_time_eq(nonce, expected_nonce) {
        return Err(SnsFailure::Verify);
    }
    let sub = claims
        .get("sub")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(SnsFailure::Verify)?
        .to_string();
    let email = claims
        .get("email")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    // Providers report this as a JSON boolean; accept the
    // string form too rather than failing a legitimate token.
    let email_verified = match claims.get("email_verified") {
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::String(flag)) => flag == "true",
        _ => false,
    };
    let name = claims
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    Ok(SnsClaims {
        sub,
        email_verified: email.is_some() && email_verified,
        email,
        name,
    })
}

/// One cached JWKS document.
struct JwksCache {
    fetched_at: Instant,
    set: jsonwebtoken::jwk::JwkSet,
}

static JWKS_CACHE: OnceLock<Mutex<HashMap<&'static str, JwksCache>>> = OnceLock::new();

/// Finds the signing key for one key id: a fresh cached set
/// answers directly; a missing key id (rotation) or a stale
/// set refetches once.
async fn fetch_jwk(provider: SnsProvider, kid: &str) -> Result<jsonwebtoken::jwk::Jwk, SnsFailure> {
    if let Some(jwk) = cached_jwk(provider, kid) {
        return Ok(jwk);
    }
    let response = reqwest::get(provider.endpoints().jwks)
        .await
        .map_err(|_| SnsFailure::Verify)?;
    if !response.status().is_success() {
        return Err(SnsFailure::Verify);
    }
    let set: jsonwebtoken::jwk::JwkSet = response.json().await.map_err(|_| SnsFailure::Verify)?;
    let found = set.find(kid).cloned();
    let cache = JWKS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            provider.slug(),
            JwksCache {
                fetched_at: Instant::now(),
                set,
            },
        );
    }
    found.ok_or(SnsFailure::Verify)
}

/// The cached key for one provider and key id, when the set
/// is still within its TTL.
fn cached_jwk(provider: SnsProvider, kid: &str) -> Option<jsonwebtoken::jwk::Jwk> {
    let cache = JWKS_CACHE.get()?;
    let guard = cache.lock().ok()?;
    let entry = guard.get(provider.slug())?;
    if entry.fetched_at.elapsed() > JWKS_TTL {
        return None;
    }
    entry.set.find(kid).cloned()
}

/// A 256-bit random value, base64url-encoded: the state,
/// nonce, and PKCE verifier all take this shape.
fn random_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen_range(0..=255)).collect();
    base64url(&bytes)
}

/// The PKCE S256 challenge for one verifier.
fn pkce_challenge(verifier: &str) -> String {
    base64url(&sha2::Sha256::digest(verifier.as_bytes()))
}

/// base64url encoding without padding (RFC 4648 section 5),
/// hand-rolled like the TOTP base32: the alphabet is URL-safe
/// by construction, so state and nonce need no further
/// encoding.
fn base64url(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        let n = u32::from_be_bytes([0, first, second, third]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

/// Percent-encodes a query value: the RFC 3986 unreserved
/// set passes through, everything else becomes %XX.
fn url_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// What a verified provider identity resolves to at the
/// sign-in callback: the branch the controller finishes
/// with session writes and rendering.
pub enum SignInResolution {
    /// A known identity whose owner is an active member:
    /// sign them in. Boxed: the member row dwarfs the other
    /// variants, and this resolves once per callback.
    SignedIn(Box<User>),
    /// The provider-verified email already belongs to an
    /// account: never merge silently — the owner links it
    /// after a password sign-in and confirmation.
    LinkRequired {
        /// The verified email the account owns.
        email: String,
    },
    /// A new registration on the provider-verified email:
    /// the born-verified pending row's id, carrying the
    /// registration through the shared details/confirm steps.
    RegisterVerified(i64),
    /// No usable email (LINE without the email permission,
    /// Yahoo before its review, an unverified address): the
    /// registration asks for one and verifies it with this
    /// store's own code.
    NeedsEmail,
    /// A database failure (already logged at its stage).
    Failed,
}

/// Resolves a verified provider identity at the sign-in
/// callback: a known identity signs its owner in; a new
/// one branches on the provider-verified email.
pub async fn resolve_sign_in(
    pool: &PgPool,
    provider: SnsProvider,
    claims: &SnsClaims,
) -> SignInResolution {
    match user_identity_repository::find_by_provider_sub(pool, provider.slug(), &claims.sub).await {
        Ok(Some(identity)) => {
            return match user_repository::find_by_id(pool, identity.user_id).await {
                Ok(Some(user)) if user.withdrawn_at.is_none() => {
                    SignInResolution::SignedIn(Box::new(user))
                }
                _ => SignInResolution::Failed,
            };
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("sns identity lookup failed: {e}");
            return SignInResolution::Failed;
        }
    }
    let Some(email) = claims.email.clone().filter(|_| claims.email_verified) else {
        return SignInResolution::NeedsEmail;
    };
    match user_repository::id_for_email(pool, &email).await {
        Ok(Some(_)) => SignInResolution::LinkRequired { email },
        Ok(None) => match pending_registration_repository::create_verified(pool, &email).await {
            Ok(pending_id) => SignInResolution::RegisterVerified(pending_id),
            Err(e) => {
                tracing::error!("sns pending creation failed: {e}");
                SignInResolution::Failed
            }
        },
        Err(e) => {
            tracing::error!("sns email lookup failed: {e}");
            SignInResolution::Failed
        }
    }
}

/// How linking a verified identity to a signed-in member
/// ends; the controller answers each with its flash.
pub enum LinkOutcome {
    /// The member already has an identity for this provider.
    AlreadyLinkedProvider,
    /// The identity already belongs to another member.
    AccountUsed,
    /// The identity is linked to the member now.
    Linked,
    /// A database failure (already logged at its stage).
    Failed,
}

/// Links a verified identity to the member who started a
/// link flow, refusing both a second identity for the same
/// provider and an identity another member already holds.
pub async fn link_identity(
    pool: &PgPool,
    user_id: i64,
    provider: SnsProvider,
    claims: &SnsClaims,
) -> LinkOutcome {
    let mine = identities_for(pool, user_id).await;
    if mine.iter().any(|row| row.provider == provider.slug()) {
        return LinkOutcome::AlreadyLinkedProvider;
    }
    match user_identity_repository::find_by_provider_sub(pool, provider.slug(), &claims.sub).await {
        Ok(Some(_)) => LinkOutcome::AccountUsed,
        Ok(None) => match user_identity_repository::insert(
            pool,
            user_id,
            provider.slug(),
            &claims.sub,
            claims.email.as_deref(),
        )
        .await
        {
            Ok(_) => LinkOutcome::Linked,
            Err(e) => {
                tracing::error!("sns identity insert failed: {e}");
                LinkOutcome::Failed
            }
        },
        Err(e) => {
            tracing::error!("sns identity lookup failed: {e}");
            LinkOutcome::Failed
        }
    }
}

/// How completing a parked link request ends; the
/// controller answers each with its flash.
pub enum PendingLinkOutcome {
    /// The identity is linked to the member now.
    Linked,
    /// The identity was already the member's own.
    AlreadyMine,
    /// The identity already belongs to another member.
    UsedByOther,
    /// A database failure (already logged at its stage).
    Failed,
}

/// Completes the link the callback parked, for the account
/// owner who has since signed in with a password. The
/// identity must still be unclaimed.
pub async fn complete_pending_link(
    pool: &PgPool,
    user_id: i64,
    provider: SnsProvider,
    provider_user_id: &str,
    provider_email: &str,
) -> PendingLinkOutcome {
    match user_identity_repository::find_by_provider_sub(pool, provider.slug(), provider_user_id)
        .await
    {
        Ok(None) => match user_identity_repository::insert(
            pool,
            user_id,
            provider.slug(),
            provider_user_id,
            Some(provider_email),
        )
        .await
        {
            Ok(_) => PendingLinkOutcome::Linked,
            Err(e) => {
                tracing::error!("sns link-pending insert failed: {e}");
                PendingLinkOutcome::Failed
            }
        },
        Ok(Some(identity)) if identity.user_id == user_id => PendingLinkOutcome::AlreadyMine,
        Ok(Some(_)) => PendingLinkOutcome::UsedByOther,
        Err(e) => {
            tracing::error!("sns link-pending lookup failed: {e}");
            PendingLinkOutcome::Failed
        }
    }
}

/// How removing one of a member's links ends; the
/// controller answers each with its flash.
pub enum UnlinkOutcome {
    /// The link is removed.
    Unlinked,
    /// A passwordless account may not remove its only
    /// sign-in method: it sets a password first.
    SoleMethod,
    /// A database failure (already logged at its stage).
    Failed,
}

/// Removes one of a member's provider links, guarding the
/// sole-sign-in-method rule for passwordless accounts.
pub async fn unlink(pool: &PgPool, user: &User, provider: SnsProvider) -> UnlinkOutcome {
    if !user.has_password {
        match user_identity_repository::count_for_user(pool, user.id).await {
            Ok(count) if count <= 1 => return UnlinkOutcome::SoleMethod,
            Ok(_) => {}
            Err(e) => {
                tracing::error!("sns identity count failed: {e}");
                return UnlinkOutcome::Failed;
            }
        }
    }
    match user_identity_repository::delete(pool, user.id, provider.slug()).await {
        Ok(_) => UnlinkOutcome::Unlinked,
        Err(e) => {
            tracing::error!("sns identity delete failed: {e}");
            UnlinkOutcome::Failed
        }
    }
}

/// One member's linked identities. A read failure logs and
/// reads as no links: the settings page and the link flow
/// both degrade to their empty state rather than fail.
pub async fn identities_for(pool: &PgPool, user_id: i64) -> Vec<UserIdentity> {
    user_identity_repository::list_for_user(pool, user_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("sns identity list failed: {e}");
            Vec::new()
        })
}

/// Whether an account already owns `email` — the SNS
/// registration's link-instead-of-merge check.
///
/// # Errors
///
/// [`AppError::Db`] when the lookup fails (logged at the
/// source with its stage).
pub async fn email_has_account(pool: &PgPool, email: &str) -> Result<bool, AppError> {
    user_repository::id_for_email(pool, email)
        .await
        .map(|id| id.is_some())
        .map_err(|e| AppError::db("sns email lookup", e))
}

/// Drops a pending registration row (the SNS details step
/// retiring a row that became a link request). Best-effort,
/// as the flow has always treated it: the row also expires
/// on its own.
pub async fn discard_pending(pool: &PgPool, pending_id: i64) {
    let _ = pending_registration_repository::delete_by_id(pool, pending_id).await;
}

#[cfg(test)]
mod tests {
    use super::{base64url, pkce_challenge, url_encode};

    #[test]
    fn base64url_uses_the_url_safe_alphabet_without_padding() {
        assert_eq!(base64url(&[]), "");
        assert_eq!(base64url(&[0xfb, 0xff, 0xfe]), "-__-");
        assert_eq!(base64url(b"hello"), "aGVsbG8");
    }

    #[test]
    fn pkce_challenge_matches_the_rfc_7636_vector() {
        // The worked example of RFC 7636 appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn url_encode_keeps_unreserved_and_escapes_the_rest() {
        assert_eq!(url_encode("openid email"), "openid%20email");
        assert_eq!(
            url_encode("https://example.jp/auth/line/callback"),
            "https%3A%2F%2Fexample.jp%2Fauth%2Fline%2Fcallback"
        );
    }
}
