//! MCP OAuth 2.1 token validation: the resource server's
//! half of the Phase 4 authorization model.
//!
//! The authorization server is external (Auth0); this
//! service only *validates* the access tokens it issues.
//! A token is trusted when its RS256 signature verifies
//! against the issuer's JWKS, its `iss` is the configured
//! issuer and its `aud` the configured audience — the
//! three checks the MCP authorization profile makes
//! mandatory. The validated token is never forwarded
//! anywhere: provider calls keep using the store's own
//! server-side keys (no token passthrough).

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::config::CONFIG;

/// How long a fetched JWKS document is trusted before it
/// is fetched again. Ten minutes keeps a key rotation's
/// blind window short while the per-request cost stays at
/// zero; an unknown `kid` refetches early, once, so a
/// rotation is picked up on the first token that uses it.
const JWKS_TTL: Duration = Duration::from_secs(600);

/// The JWKS fetch's timeout: validation sits on the
/// request path, so a hung issuer must fail closed fast,
/// never stall a tool call.
const JWKS_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// The shortest gap between two JWKS fetches. An unknown
/// `kid` refetches early (see [`JWKS_TTL`]), but a stream
/// of tokens with invented key ids must not turn the store
/// into a fetch amplifier against the issuer: inside the
/// window, an uncached key id simply fails closed.
const JWKS_REFETCH_COOLDOWN: Duration = Duration::from_secs(60);

/// Who a validated access token speaks for: the subject
/// and the scopes it was granted. Carries no token
/// material — the token itself is dropped at validation —
/// and deliberately does not derive `Debug`, so a stray
/// log line can never print claim values.
pub struct OAuthIdentity {
    /// The token's subject (`sub` claim).
    pub subject: String,
    /// The granted scopes, split out of the `scope` claim.
    pub scopes: Vec<String>,
}

/// The claims this server acts on. Everything else in the
/// token is ignored, on the SNS verification's discipline:
/// only what a layer may act on is ever read out.
#[derive(Deserialize)]
struct AccessTokenClaims {
    /// Subject: the principal the token speaks for.
    sub: String,
    /// Granted scopes: a space-separated string in the
    /// OAuth profile; some issuers send a JSON array
    /// instead, and both shapes mean the same list.
    #[serde(default, rename = "scope", deserialize_with = "scope_claim")]
    scopes: Vec<String>,
}

/// Deserialises the `scope` claim in either of its wire
/// shapes (a space-separated string, or an array of
/// strings) into the scope list.
fn scope_claim<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ScopeClaim {
        Text(String),
        List(Vec<String>),
    }
    Ok(match Option::<ScopeClaim>::deserialize(deserializer)? {
        Some(ScopeClaim::Text(text)) => split_scopes(&text),
        Some(ScopeClaim::List(list)) => list,
        None => Vec::new(),
    })
}

/// Splits a space-separated scope string into its scopes.
fn split_scopes(text: &str) -> Vec<String> {
    text.split_whitespace().map(ToString::to_string).collect()
}

/// The one cached JWKS document: when it was fetched and
/// the parsed key set. A single issuer is configured, so
/// a single entry is the whole cache.
static JWKS_CACHE: Mutex<Option<(Instant, Jwks)>> = Mutex::new(None);

/// When the last JWKS fetch was attempted, success or
/// failure: the refetch cooldown's clock.
static JWKS_LAST_FETCH: Mutex<Option<Instant>> = Mutex::new(None);

/// Type alias for the cached key set, so the cache's
/// shape names what it holds.
type Jwks = JwkSet;

/// Validates one bearer access token against the
/// configured issuer and audience.
///
/// # Errors
/// Returns `Err(())` for every failure — inactive OAuth,
/// malformed token, unknown or unusable signing key,
/// bad signature, wrong issuer, wrong audience, expired,
/// missing subject. The unit error is the no-oracle
/// contract, as with API key authentication: the caller
/// learns only that the token is not accepted, never
/// which check refused it.
#[expect(
    clippy::result_unit_err,
    reason = "the unit error is the no-oracle contract: there is deliberately no error value to return"
)]
pub async fn validate_access_token(token: &str) -> Result<OAuthIdentity, ()> {
    let issuer = CONFIG.mcp_oauth_issuer().ok_or(())?;
    let audience = CONFIG.mcp_oauth_audience().ok_or(())?;
    let header = jsonwebtoken::decode_header(token).map_err(|_| ())?;
    // The algorithm is pinned, never taken from the token:
    // the issuer signs RS256, and letting the header choose
    // is the classic JOSE algorithm-confusion footgun (the
    // SNS verification's rule, applied here too).
    if header.alg != Algorithm::RS256 {
        return Err(());
    }
    let kid = header.kid.ok_or(())?;
    let jwk = fetch_jwk(&issuer, &kid).await?;
    let (modulus, exponent) = match &jwk.algorithm {
        jsonwebtoken::jwk::AlgorithmParameters::RSA(params) => (&params.n, &params.e),
        _ => return Err(()),
    };
    let key = DecodingKey::from_rsa_components(modulus, exponent).map_err(|_| ())?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[audience.as_str()]);
    validation.set_issuer(&[issuer.as_str()]);
    // Expiry, issuer and audience are all required claims,
    // not merely checked when present.
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    let data =
        jsonwebtoken::decode::<AccessTokenClaims>(token, &key, &validation).map_err(|_| ())?;
    if data.claims.sub.is_empty() {
        return Err(());
    }
    Ok(OAuthIdentity {
        subject: data.claims.sub,
        scopes: data.claims.scopes,
    })
}

/// Finds the signing key for one key id: a fresh cached
/// set answers directly; a missing key id (a rotation) or
/// a stale set refetches the document, at most once per
/// refetch cooldown — inside the window an unknown key id
/// fails closed instead of fetching again.
async fn fetch_jwk(issuer: &str, kid: &str) -> Result<jsonwebtoken::jwk::Jwk, ()> {
    if let Some(jwk) = cached_jwk(kid) {
        return Ok(jwk);
    }
    {
        // The cooldown gate: the attempt is recorded
        // before fetching, so a failed fetch counts
        // against the window too — a dead issuer is not
        // hammered either. A poisoned lock fails closed,
        // like every other refusal in this module.
        let mut last_fetch = JWKS_LAST_FETCH.lock().map_err(|_| ())?;
        if matches!(*last_fetch, Some(at) if at.elapsed() < JWKS_REFETCH_COOLDOWN) {
            return Err(());
        }
        *last_fetch = Some(Instant::now());
    }
    let url = format!("{}/.well-known/jwks.json", issuer.trim_end_matches('/'));
    let response = http_client().get(url).send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let set: Jwks = response.json().await.map_err(|_| ())?;
    let found = set.find(kid).cloned();
    if let Ok(mut cache) = JWKS_CACHE.lock() {
        *cache = Some((Instant::now(), set));
    }
    found.ok_or(())
}

/// The cached key for one key id, when the set is still
/// within its TTL. A poisoned lock reads as an empty
/// cache: validation then refetches, it never panics.
fn cached_jwk(kid: &str) -> Option<jsonwebtoken::jwk::Jwk> {
    let cache = JWKS_CACHE.lock().ok()?;
    let (fetched_at, set) = cache.as_ref()?;
    if fetched_at.elapsed() > JWKS_TTL {
        return None;
    }
    set.find(kid).cloned()
}

/// The shared HTTP client for issuer fetches, built once
/// with the fetch timeout (the agent chat client's
/// pattern).
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(JWKS_FETCH_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_strings_split_on_whitespace() {
        assert_eq!(
            split_scopes("cart:write orders:read"),
            vec!["cart:write".to_string(), "orders:read".to_string()]
        );
        assert!(split_scopes("").is_empty());
        assert!(split_scopes("  catalog:read  ").len() == 1);
    }

    #[test]
    fn scope_claim_accepts_both_wire_shapes() {
        // Mirrors AccessTokenClaims' field shape exactly:
        // the JSON `scope` claim, in either wire form.
        #[derive(Deserialize)]
        struct Probe {
            #[serde(default, rename = "scope", deserialize_with = "scope_claim")]
            scopes: Vec<String>,
        }
        let from_text: Probe =
            serde_json::from_value(serde_json::json!({"scope": "cart:write orders:write"}))
                .expect("text shape");
        assert_eq!(from_text.scopes.len(), 2);
        let from_list: Probe = serde_json::from_value(serde_json::json!({"scope": ["cart:write"]}))
            .expect("list shape");
        assert_eq!(from_list.scopes, vec!["cart:write".to_string()]);
        let absent: Probe = serde_json::from_value(serde_json::json!({})).expect("absent");
        assert!(absent.scopes.is_empty());
    }
}
