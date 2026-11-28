//! Session infrastructure for Kaimoru.
//!
//! Owns the session itself: every key the record may hold, the
//! current-user extractors and the CSRF token helpers. Sign-up and
//! login business rules live in [`crate::services::member_account_service`].

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::Redirect,
};
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::models::{AdminIdentity, User};
use crate::repositories::{admin::admin_repository, user_repository};
use crate::util::hmac_util::hex_encode;

/// Session key for the logged in user's ID: the durable identity
/// the session is bound to. The record also carries flow state
/// (the CSRF token and its credential fingerprint, pending-flow
/// IDs, the one-shot flash, checkout state) — never a second
/// identity.
pub const SESSION_USER_ID: &str = "user_id";

/// Session key for the signed-in admin's ID, held only by the
/// separate admin session (see [`crate::session_store::init_admin_session_layer`]):
/// the two identities never share a session record.
pub const SESSION_ADMIN_ID: &str = "admin_id";

/// Session key for the registration in progress; cleared when the
/// account is created.
pub const SESSION_PENDING_ID: &str = "pending_registration_id";

/// Session key for the password reset in progress; cleared when
/// it completes.
pub const SESSION_RESET_ID: &str = "password_reset_id";

/// Session key for the email change in progress; cleared when
/// it completes.
pub const SESSION_EMAIL_CHANGE_ID: &str = "email_change_id";

/// Session key for the SNS flow in progress (SNS login): the
/// provider, state, nonce, PKCE verifier, destination and link
/// intent of one authorization round trip. The callback
/// removes it before use, so a state can never be replayed.
pub const SESSION_SNS_FLOW: &str = "sns_flow";

/// Session key for the SNS registration profile: the verified
/// provider identity waiting at the details step.
pub const SESSION_SNS_PENDING: &str = "sns_pending";

/// Session key for the SNS link-pending marker: a provider
/// identity whose email matched an existing account, waiting
/// for the owner's password sign-in and confirmation.
pub const SESSION_SNS_LINK_PENDING: &str = "sns_link_pending";

/// Session key for the withdrawal code challenge in progress
/// (passwordless accounts prove ownership by email code);
/// cleared when the withdrawal completes.
pub const SESSION_WITHDRAWAL_ID: &str = "withdrawal_id";

/// Session key for the member whose password has verified but
/// whose two-factor challenge is still outstanding. This marker
/// alone never authenticates: [`SESSION_USER_ID`] is inserted
/// only when the challenge completes.
pub const SESSION_TOTP_PENDING_USER: &str = "totp_pending_user_id";

/// Session key for when the two-factor challenge began, as a
/// Unix timestamp: a challenge older than the pending window
/// is discarded, never completed.
pub const SESSION_TOTP_PENDING_SINCE: &str = "totp_pending_since";

/// Session key for the validated `next` return path chosen at
/// the password step, carried through the two-factor challenge.
pub const SESSION_TOTP_PENDING_NEXT: &str = "totp_pending_next";

/// Session key for the ADMIN whose password has verified
/// but whose two-factor challenge is still outstanding
///. The admin session is a separate
/// cookie from the member session, so the marker can never
/// cross over; like the member marker, it alone never
/// authenticates — `SESSION_ADMIN_ID` is inserted only
/// when the challenge completes.
pub const SESSION_ADMIN_TOTP_PENDING: &str = "admin_totp_pending_id";

/// Session key for when the admin two-factor challenge
/// began, as a Unix timestamp: a challenge older than the
/// pending window is discarded, never completed.
pub const SESSION_ADMIN_TOTP_PENDING_SINCE: &str = "admin_totp_pending_since";

/// Session key for the admin challenge attempts used, so
/// a pending login cannot be ground through the code
/// space.
pub const SESSION_ADMIN_TOTP_ATTEMPTS: &str = "admin_totp_attempts";

/// Session key for the admin settings-proof attempts used
/// (recovery-code regeneration), so a signed-in session
/// cannot grind through the code space either.
pub const SESSION_ADMIN_TOTP_PROOF_ATTEMPTS: &str = "admin_totp_proof_attempts";

/// Session key for the two-factor enrolment secret awaiting
/// confirmation. It lives only in the session until a code
/// proves the member's app holds it; the users row is written
/// at that moment, never before.
pub const SESSION_TOTP_SETUP_SECRET: &str = concat!("totp_setup_", "secret");

/// Session key for the enrolment confirmation attempts used,
/// so a pending secret cannot be brute forced in place.
pub const SESSION_TOTP_SETUP_ATTEMPTS: &str = "totp_setup_attempts";

/// Session key for the member settings-proof attempts used
/// (recovery-code regeneration), so a signed-in session
/// cannot grind through the code space either.
pub const SESSION_TOTP_PROOF_ATTEMPTS: &str = "totp_proof_attempts";

/// Session key for the CSRF token of this session.
const SESSION_CSRF_TOKEN: &str = "csrf_token";

/// Session key for the CSRF credential fingerprint: a SHA-256
/// hash of the signed-in principal's credential fields, stored
/// beside the token so the CSRF middleware can notice when the
/// credentials themselves changed (see [`crate::middleware::csrf`]). Only
/// the hash is stored, never a credential value.
pub const SESSION_CSRF_FINGERPRINT: &str = "csrf_fingerprint";

/// Session key for a one-time message shown after a redirect.
pub const SESSION_FLASH: &str = "flash_message";

/// Session key holding the guest cart owner token: a random value
/// identifying the guest's cart rows, with no personal meaning.
pub const SESSION_CART_OWNER: &str = "cart_owner";

/// Session key holding the variant IDs the visitor's latest
/// agent run refused for budget: the only variants the
/// add-anyway endpoint may add without a budget check, so a
/// crafted post cannot mint agent lines for arbitrary items.
pub const SESSION_AGENT_OVER_BUDGET: &str = "agent_over_budget";

/// Session key holding the variant IDs of the visitor's
/// latest chat proposal the shopper has already applied to
/// the cart (per-pick or apply-all): their pick cards render
/// the in-cart state instead of the apply button. A new
/// proposal resets the list — it describes one proposal,
/// not the cart.
pub const SESSION_AGENT_APPLIED: &str = "agent_applied";

/// Session key holding the agent chat conversation: the thread entries and the brief the conversation
/// has accumulated. Display and shopping state only — the cart
/// itself stays in its own table, as with every other flow.
pub const SESSION_AGENT_CHAT: &str = "agent_chat";
/// One-shot explanation for the agent approval card
/// an approval POST whose outcome the card's
/// own state cannot show (the hourly cap refused it) leaves
/// its message here; the next card render takes it.
pub const SESSION_AGENT_APPROVAL_NOTICE: &str = "agent_approval_notice";

/// Session key holding the in-chat settlement in progress
/// which approval record it walks, its
/// current step, and the delivery state chosen so far. The
/// settlement's authority is the approval record in the
/// database; this is only the walk's place in it, so a
/// page reload resumes the step the server left off.
pub const SESSION_AGENT_SETTLEMENT: &str = "agent_settlement";

/// Session key holding the cart fingerprint whose checkout
/// offer the visitor dismissed with 買い物を続ける
/// the offer stays hidden while the cart's
/// fingerprint matches, and returns the moment the cart —
/// and therefore the fingerprint — changes.
pub const SESSION_AGENT_SETTLEMENT_DISMISSED: &str = "agent_settlement_dismissed";

/// Session key holding the in-progress checkout form (delivery
/// details and applied coupon) between the two checkout POSTs.
pub const SESSION_CHECKOUT: &str = "checkout_flow";

/// Session key holding the last placed order's ID, so the
/// completion page and double-submit handling can find it.
pub const SESSION_LAST_ORDER: &str = "last_order_id";

/// Session key holding the confirmation page's figures
/// fingerprint: stamped when the confirmation renders,
/// consumed by the placement POST, which refuses to place at
/// figures the shopper never saw.
pub const SESSION_CONFIRM_FINGERPRINT: &str = "confirm_fingerprint";

/// One-shot notice on the payment step, shown once when a
/// placement was refused because the confirmed figures moved;
/// consumed by the next render of that page.
pub const SESSION_CHECKOUT_NOTICE: &str = "checkout_notice";

/// Session key for the address-book save note, shown once on
/// the completion page when a requested save was skipped at
/// the book's cap (Address Book spec §5.3).
pub const SESSION_ADDRESS_SAVE_NOTE: &str = "address_save_note";

/// Session key for the one-time inquiry reference shown
/// on the contact completion page.
pub const SESSION_INQUIRY_REF: &str = "inquiry_reference";

/// One-shot notice on the member order detail (a refused
/// cancellation's explanation), consumed by the next
/// render of that page.
pub const SESSION_ORDER_NOTICE: &str = "order_notice";

/// One-shot notice on the cart page (the reorder's
/// added/skipped summary), consumed by the next full cart
/// render. The cart page deliberately never renders the
/// stored header flash — its confirmations are
/// response-only toasts — so flows landing here with
/// something to report use this page-scoped key, the same
/// pattern as [`SESSION_ORDER_NOTICE`].
pub const SESSION_CART_NOTICE: &str = "cart_notice";

/// Reads the session the session layer attached to this request.
fn session_from(parts: &Parts) -> Option<Session> {
    parts.extensions.get::<Session>().cloned()
}

/// The logged-in user, for handlers that may also serve guests:
/// `None` when the session carries no valid user ID.
pub struct OptionalCurrentUser(pub Option<User>);

impl FromRequestParts<PgPool> for OptionalCurrentUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &PgPool,
    ) -> Result<Self, Self::Rejection> {
        let Some(session) = session_from(parts) else {
            return Ok(Self(None));
        };
        Ok(Self(current_user(state, &session).await))
    }
}

/// The logged in user, for member-only handlers.
/// Guests are redirected to the login page instead of seeing an error.
pub struct CurrentUser(pub User);

impl FromRequestParts<PgPool> for CurrentUser {
    type Rejection = Redirect;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &PgPool,
    ) -> Result<Self, Self::Rejection> {
        match OptionalCurrentUser::from_request_parts(parts, state).await {
            Ok(OptionalCurrentUser(Some(user))) => Ok(Self(user)),
            _ => {
                // Redirect guests to login with a return path; the
                // login side validates it against its allowlist.
                let path = parts.uri.path();
                if path == "/" {
                    Err(Redirect::to("/login"))
                } else {
                    Err(Redirect::to(&format!("/login?next={path}")))
                }
            }
        }
    }
}

/// Loads the logged in user for a session, if any.
/// A failed lookup means a guest, never an error.
pub async fn current_user(pool: &PgPool, session: &Session) -> Option<User> {
    let user_id = session.get::<i64>(SESSION_USER_ID).await.ok()??;
    // A lookup failure resolves to "guest" for the caller, but is
    // logged: swallowing it once hid a broken database behind
    // pages that merely looked logged-out.
    match user_repository::find_by_id(pool, user_id).await {
        // A withdrawn member's row is a tombstone: it must
        // never authenticate, even if a session still names
        // it (the withdrawal ceremony's CSRF rotation can
        // rewrite the session after revocation).
        Ok(user) => user.filter(|u| u.withdrawn_at.is_none()),
        Err(e) => {
            tracing::error!(user_id, "current user lookup failed: {e}");
            None
        }
    }
}

/// Takes the checkout flow out of the session, or `None` when absent.
///
/// Only the storage lives here, with the session keys; the flow's
/// shape and rules belong to `order_service`, which never touches
/// a `Session`. Taking (not loading) means a double submission of
/// the confirm form cannot place the same order twice.
pub async fn take_checkout_flow<T>(session: &Session) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    session.remove(SESSION_CHECKOUT).await.ok().flatten()
}

/// Loads the checkout flow from the session, or `None` when
/// absent or unreadable.
pub async fn load_checkout_flow<T>(session: &Session) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    session.get(SESSION_CHECKOUT).await.ok().flatten()
}

/// Stores the checkout flow in the session, best effort.
///
/// A store failure is logged, not raised: the next step revalidates
/// from an empty flow and lands back on the cart page instead of
/// failing mid-checkout with a 500.
pub async fn save_checkout_flow<T>(session: &Session, flow: &T)
where
    T: serde::Serialize + Clone,
{
    if let Err(error) = session.insert(SESSION_CHECKOUT, flow.clone()).await {
        tracing::error!("failed to store checkout flow in session: {error}");
    }
}

/// The signed-in admin, for back-office handlers.
///
/// Anyone else — guests and members alike — is redirected to the
/// admin login: the admin session is a separate cookie, so a member
/// session carries no admin ID here. No `next` return path: after
/// login the admin lands on the dashboard.
pub struct CurrentAdmin(pub AdminIdentity);

impl FromRequestParts<PgPool> for CurrentAdmin {
    type Rejection = Redirect;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &PgPool,
    ) -> Result<Self, Self::Rejection> {
        let Some(session) = session_from(parts) else {
            return Err(Redirect::to("/admin/login"));
        };
        match current_admin(state, &session).await {
            Some(admin) => Ok(Self(admin)),
            None => Err(Redirect::to("/admin/login")),
        }
    }
}

/// Loads the signed-in admin for a session, if any.
///
/// A failed lookup resolves to "not an admin"; a database error
/// is logged rather than hidden.
pub async fn current_admin(pool: &PgPool, session: &Session) -> Option<AdminIdentity> {
    let admin_id = session.get::<i64>(SESSION_ADMIN_ID).await.ok()??;
    match admin_repository::find_admin_by_id(pool, admin_id).await {
        // A disabled account resolves as signed out on every
        // request, so deactivation takes effect immediately —
        // no session-store surgery is needed to cut access.
        Ok(admin) => admin.filter(|a| a.is_active),
        Err(e) => {
            tracing::error!(admin_id, "current admin lookup failed: {e}");
            None
        }
    }
}

/// Returns the CSRF token for this session, creating one on first use.
/// The token lives in the session record and rotates with it, so a
/// form stolen from another session can never be replayed here.
pub async fn csrf_token(session: &Session) -> String {
    if let Ok(Some(token)) = session.get::<String>(SESSION_CSRF_TOKEN).await {
        return token;
    }
    let token = generate_token();
    let _ = session.insert(SESSION_CSRF_TOKEN, token.clone()).await;
    token
}

/// Checks a submitted CSRF token against the session's token.
pub async fn csrf_valid(session: &Session, submitted: &str) -> bool {
    match session.get::<String>(SESSION_CSRF_TOKEN).await {
        Ok(Some(token)) => !submitted.is_empty() && constant_time_eq(&token, submitted),
        _ => false,
    }
}

/// Replaces the session's CSRF token with a fresh one and
/// returns it.
///
/// Rotation replaces rather than removes: the caller (the CSRF
/// middleware) still has a response to write the token into, and
/// a stored token keeps every later read consistent — a removed
/// token would be recreated lazily, possibly as two different
/// values under racing requests.
pub async fn csrf_rotate(session: &Session) -> String {
    let token = generate_token();
    let _ = session.insert(SESSION_CSRF_TOKEN, token.clone()).await;
    token
}

/// The credential state of the principal a session is signed
/// in as, as the CSRF middleware needs it (see [`crate::middleware::csrf`]).
#[derive(Debug, PartialEq)]
pub enum CredentialState {
    /// The principal's current credential fingerprint.
    Fingerprint(String),
    /// The member row is a withdrawal tombstone: the request is
    /// tearing this session down, and nothing may be written
    /// back into the record it deletes.
    TornDown,
}

/// Loads the credential state of the principal this session is
/// signed in as.
///
/// `None` when the session holds no principal, the principal's
/// row is gone, or the load failed — failures are logged here,
/// so a caller can fail toward rotation without inventing a
/// reason.
///
/// The fingerprint is a SHA-256 hash over the principal's
/// credential fields as they stand in the database: a member's
/// password hash, email and two-factor enrolment (the columns
/// behind [`crate::repositories::user_repository::credential_fields`]),
/// an admin's password hash and email. Any credential change
/// rewrites one of those fields, so the fingerprint moves with
/// the change itself and no flow has to announce it — a handler
/// that replaces a credential in a way nobody wired up still
/// rotates. Recovery codes are not an input: they live in
/// their own table, so reissuing them does not move the
/// fingerprint. Only the hash is stored and compared; a
/// credential value never leaves the row.
pub async fn credential_state(pool: &PgPool, session: &Session) -> Option<CredentialState> {
    if let Ok(Some(user_id)) = session.get::<i64>(SESSION_USER_ID).await {
        return match user_repository::credential_fields(pool, user_id).await {
            Ok(Some((password_hash, email, totp_secret, totp_enabled_at, withdrawn_at))) => {
                if withdrawn_at.is_some() {
                    Some(CredentialState::TornDown)
                } else {
                    let totp_enabled = totp_enabled_at.map(|at| at.to_rfc3339());
                    Some(CredentialState::Fingerprint(derive_credential_fingerprint(
                        &[
                            &password_hash,
                            &email,
                            totp_secret.as_deref().unwrap_or(""),
                            totp_enabled.as_deref().unwrap_or(""),
                        ],
                    )))
                }
            }
            Ok(None) => None,
            Err(e) => {
                tracing::error!(user_id, "credential fingerprint load failed: {e}");
                None
            }
        };
    }
    if let Ok(Some(admin_id)) = session.get::<i64>(SESSION_ADMIN_ID).await {
        return match admin_repository::find_admin_auth_by_id(pool, admin_id).await {
            Ok(Some(admin)) => Some(CredentialState::Fingerprint(derive_credential_fingerprint(
                &[&admin.password_hash, &admin.email],
            ))),
            Ok(None) => None,
            Err(e) => {
                tracing::error!(admin_id, "credential fingerprint load failed: {e}");
                None
            }
        };
    }
    None
}

/// Derives the one credential fingerprint from a principal's
/// credential fields: SHA-256 over the fields, each prefixed
/// with its length so no two ways of splitting the same bytes
/// into fields can collide, hex encoded. This is the one
/// derivation; the loads in [`credential_state`] are the one
/// place its inputs are listed.
fn derive_credential_fingerprint(fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    hex_encode(&hasher.finalize())
}

/// Compares two strings in constant time, without leaking where
/// they differ. Length is not secret, so a length mismatch may
/// return early.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Returns the client IP for throttling and rate limits.
///
/// The direct peer address by default. Behind the ALB the peer is
/// the load balancer for everyone, which would make per-IP limits
/// global, so with `TRUST_PROXY_HEADERS=1` (AWS deployment only,
/// where the ALB is the sole ingress) the first `X-Forwarded-For`
/// address is used. Never enable that flag where clients can reach
/// the app directly: the header is client-controlled and spoofable.
pub fn client_ip(addr: &std::net::SocketAddr, headers: &axum::http::HeaderMap) -> String {
    if trust_proxy_headers()
        && let Some(first) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
    {
        return first.trim().to_string();
    }
    addr.ip().to_string()
}

/// Whether `TRUST_PROXY_HEADERS=1` is set, read once: the
/// flag is a deployment constant, and re-reading the process
/// environment on every request is work the hot path does
/// not need.
fn trust_proxy_headers() -> bool {
    static TRUST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRUST.get_or_init(|| std::env::var("TRUST_PROXY_HEADERS").as_deref() == Ok("1"))
}

/// A random token used for CSRF protection.
fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| format!("{:02x}", rng.r#gen::<u8>()))
        .collect()
}

/// A 403 response used when a CSRF check fails.
/// The body is a fixed generic message: no request data is echoed back.
pub fn forbidden() -> (StatusCode, &'static str) {
    (
        StatusCode::FORBIDDEN,
        "不正なリクエストです。もう一度お試しください。",
    )
}

/// Reads the pending registration ID held by this session, if any.
pub async fn pending_id(session: &Session) -> Option<i64> {
    session.get::<i64>(SESSION_PENDING_ID).await.ok()?
}

/// Reads the password reset ID held by this session, if any.
pub async fn reset_id(session: &Session) -> Option<i64> {
    session.get::<i64>(SESSION_RESET_ID).await.ok()?
}

/// Reads the email change ID held by this session, if any.
pub async fn email_change_id(session: &Session) -> Option<i64> {
    session.get::<i64>(SESSION_EMAIL_CHANGE_ID).await.ok()?
}

/// The withdrawal code challenge ID parked in the session, if any.
pub async fn withdrawal_id(session: &Session) -> Option<i64> {
    session.get::<i64>(SESSION_WITHDRAWAL_ID).await.ok()?
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, derive_credential_fingerprint};

    #[test]
    fn credential_fingerprint_depends_on_every_field() {
        let base = derive_credential_fingerprint(&["hash", "a@example.jp", "", ""]);
        assert_ne!(
            base,
            derive_credential_fingerprint(&["hash2", "a@example.jp", "", ""])
        );
        assert_ne!(
            base,
            derive_credential_fingerprint(&["hash", "b@example.jp", "", ""])
        );
        assert_ne!(
            base,
            derive_credential_fingerprint(&["hash", "a@example.jp", "secret", ""])
        );
        // The same bytes split differently across fields are a
        // different credential set and must not collide.
        assert_ne!(
            derive_credential_fingerprint(&["ab", "c"]),
            derive_credential_fingerprint(&["a", "bc"])
        );
        // Deterministic: the same fields derive the same value.
        assert_eq!(
            base,
            derive_credential_fingerprint(&["hash", "a@example.jp", "", ""])
        );
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(constant_time_eq("", ""));
    }
}
