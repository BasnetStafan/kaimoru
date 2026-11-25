//! CSRF protection for the storefront and admin pages.
//!
//! One middleware owns the token's whole life cycle, so no
//! controller touches it: it checks state-changing requests
//! against the session token on the way in, and injects the token
//! into returned HTML as a hidden form field on the way out.
//!
//! Rotation follows the session and the principal's own record,
//! never the request path, and no handler carries rotation code.
//! The middleware snapshots the session's principal — the
//! signed-in member or admin — before the handler runs; when a
//! successful response leaves the principal changed, the token
//! is replaced before injection, so the answer itself carries
//! the new token in its forms. Sign-in, sign-out and sign-up
//! move the principal, so that alone covers them.
//!
//! A credential change that keeps the principal — a password,
//! email or two-factor change — rewrites a credential field on
//! the principal's own row, so the middleware watches the row
//! instead of the flows: at sign-in it stores the row's
//! credential fingerprint (see
//! [`crate::session::credential_state`]) in the session, and
//! after each state-changing request it derives the fingerprint
//! again and rotates when it moved. A flow that deletes the
//! session outright (withdrawal, admin sign-out) needs neither:
//! the token dies with the record, and the middleware writes
//! nothing back into a deleted session.
//!
//! New routes are protected by default: protection keys off the
//! method alone (see `is_safe_method`). A new credential kind
//! stored in a new column joins rotation by adding the column
//! to the fingerprint inputs behind
//! [`crate::session::credential_state`] — the one place they
//! are listed; there is no route list to extend and no flow
//! to wire.

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::session::{
    CredentialState, SESSION_ADMIN_ID, SESSION_CSRF_FINGERPRINT, SESSION_USER_ID, credential_state,
    csrf_rotate, csrf_token, csrf_valid, forbidden,
};

/// Largest form body the middleware will buffer to find the token.
///
/// The store's forms are a few kilobytes; the cap stops a hostile
/// client from making it buffer an unbounded body before any
/// handler runs.
const MAX_FORM_BYTES: usize = 64 * 1024;

/// Largest HTML body the middleware will rewrite to inject the token.
///
/// Pages are tens of kilobytes; a body over the cap is sent on
/// untouched with a warning — its forms then fail their CSRF check
/// visibly on submit, never a silent blank page.
const MAX_HTML_BYTES: usize = 1024 * 1024;

/// Largest HTML body the middleware will buffer at all while
/// deciding whether to rewrite it. Well above [`MAX_HTML_BYTES`]
/// so an oversized page can still be passed through whole.
const MAX_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Header that may carry the token instead of the form field; the
/// store's own pages always use the injected field.
const CSRF_HEADER: &str = "x-csrf-token";

/// Middleware that owns the session CSRF token for page routes.
///
/// Per request: the session token is ensured via [`csrf_token`];
/// safe methods (see `is_safe_method`) pass through; any other
/// method must present the token as the `csrf_token` form field
/// or the `x-csrf-token` header, compared in constant time by
/// [`csrf_valid`]. Returned HTML passes through injection, which
/// writes the token into every non-GET form — so controllers
/// and views never carry the token at all.
///
/// When the handler changed the session's auth state, the token
/// is rotated via [`csrf_rotate`] before injection, so a success
/// response already carries the token its forms must use next.
///
/// Mounted on the page router only (see [`crate::routes`]): an API
/// authenticates with its own credentials, not a form token, and
/// must not mount this layer.
pub async fn protect(State(pool): State<PgPool>, req: Request, next: Next) -> Response {
    // Fail closed: without a session there is nothing to check against.
    let Some(session) = req.extensions().get::<Session>().cloned() else {
        return forbidden().into_response();
    };
    let token = csrf_token(&session).await;
    // Rotation is decided by what the handler does to the session,
    // never by the request path: snapshot the auth state now and
    // compare it after the handler runs (see `finish_and_inject`).
    let before = AuthSnapshot::capture(&session).await;

    if is_safe_method(req.method()) {
        let res = next.run(req).await;
        return finish_and_inject(res, &pool, &session, &before, &token, true).await;
    }

    // A header token decides on its own: a wrong one is a failed
    // check, never a reason to fall back to the body. Checking the
    // header before any body is buffered is also what lets
    // multipart uploads through: their bodies are never read
    // here, so the form-body cap below cannot reject a
    // legitimate upload (the upload handler reads this header).
    if let Some(submitted) = req
        .headers()
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        return if csrf_valid(&session, submitted).await {
            let res = next.run(req).await;
            finish_and_inject(res, &pool, &session, &before, &token, false).await
        } else {
            forbidden().into_response()
        };
    }

    // Form posts carry the token as a field: buffer the body once,
    // check it, then replay it unchanged to the handler.
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_FORM_BYTES).await {
        Ok(bytes) => bytes,
        // A body that cannot be buffered within the cap was never
        // token-checked, so the honest answer is 413, not a CSRF
        // failure. (Uploads never reach this path: they carry
        // the header token checked above.)
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "リクエストが大きすぎます。").into_response();
        }
    };
    match submitted_token(&bytes) {
        Some(submitted) if !submitted.is_empty() && csrf_valid(&session, submitted).await => {
            let res = next
                .run(Request::from_parts(parts, Body::from(bytes)))
                .await;
            finish_and_inject(res, &pool, &session, &before, &token, false).await
        }
        _ => forbidden().into_response(),
    }
}

/// The one classification of methods that cannot change state
/// and so need no token: the standard safe methods. Everything
/// else — TRACE included, which some stacks answer by echoing
/// the request back — must present a valid token, so a method
/// this match does not name is protected by default.
fn is_safe_method(method: &Method) -> bool {
    matches!(method, &Method::GET | &Method::HEAD | &Method::OPTIONS)
}

/// The session's principal at a moment in time: who is signed
/// in on either surface.
///
/// Two snapshots bracket the handler; a difference on a success
/// or redirect response is the first rotation trigger. Sign-in,
/// sign-out and sign-up completion move a principal, so their
/// handlers need no rotation code at all; a credential change
/// that keeps the principal is caught by the fingerprint
/// comparison instead (see `finish_and_inject`).
#[derive(PartialEq)]
struct AuthSnapshot {
    /// The signed-in member's ID, when the member session has one.
    user_id: Option<i64>,
    /// The signed-in admin's ID, when the admin session has one.
    admin_id: Option<i64>,
}

impl AuthSnapshot {
    /// Reads the current principal out of the session record.
    ///
    /// Both principal keys are read on both surfaces: each
    /// surface's session only ever holds its own, so the other
    /// simply reads as absent.
    async fn capture(session: &Session) -> Self {
        Self {
            user_id: session.get::<i64>(SESSION_USER_ID).await.ok().flatten(),
            admin_id: session.get::<i64>(SESSION_ADMIN_ID).await.ok().flatten(),
        }
    }

    /// Whether either surface is signed in.
    fn is_authenticated(&self) -> bool {
        self.user_id.is_some() || self.admin_id.is_some()
    }
}

/// Decides rotation for a completed response, then writes the
/// effective token into its HTML.
///
/// Rotation replaces the token in the session before injection,
/// so the answer to a credential-changing request already
/// carries the new token; a redirect's answer has no forms, and
/// the page it leads to is injected on its own request. Only
/// success and redirect responses rotate — a refused attempt
/// changed nothing worth rotating for.
///
/// Two triggers, both read off the session and the principal's
/// own row, never the request path:
///
/// - The principal changed (sign-in, sign-out, sign-up): rotate,
///   then store the new principal's credential fingerprint as
///   the baseline for later comparisons — or drop the stored
///   fingerprint when the session signed out.
/// - The principal stayed: `reconcile_fingerprint` compares the
///   fingerprint derived from the principal's row against the
///   stored one and rotates when a credential moved under the
///   session.
///
/// A handler that deleted the session (withdrawal, admin
/// sign-out) leaves the after snapshot reading as unchanged and
/// its row torn down or untouched, so nothing is written back
/// into the deleted record: the token dies with the session.
async fn finish_and_inject(
    res: Response,
    pool: &PgPool,
    session: &Session,
    before: &AuthSnapshot,
    token: &str,
    safe_method: bool,
) -> Response {
    let mut effective = token.to_string();
    if res.status().is_success() || res.status().is_redirection() {
        let after = AuthSnapshot::capture(session).await;
        if after != *before {
            effective = csrf_rotate(session).await;
            if after.is_authenticated() {
                // Baseline for the principal this session just
                // became: stored now, so a later credential change
                // under this same principal is visible. A failed
                // load is logged inside `credential_state`; the
                // next request re-establishes the baseline.
                if let Some(CredentialState::Fingerprint(fingerprint)) =
                    credential_state(pool, session).await
                {
                    store_fingerprint(session, &fingerprint).await;
                }
            } else {
                // Sign-out: the member session record survives, so
                // the fingerprint goes with the principal.
                let _ = session.remove::<String>(SESSION_CSRF_FINGERPRINT).await;
            }
        } else if after.is_authenticated() {
            effective = reconcile_fingerprint(pool, session, safe_method, &effective).await;
        }
    }
    inject(res, &effective).await
}

/// Keeps the stored credential fingerprint in step with the
/// principal's row for a response whose principal did not move.
///
/// Returns the token to inject: the rotated one when a
/// credential changed under the session — or could not be
/// verified, which fails toward rotation, never toward keeping
/// a possibly stale token — and the arriving token otherwise.
/// Establishing a missing baseline never rotates by itself: it
/// only records what the credentials are now, so a change from
/// this point on is seen.
///
/// Safe-method responses skip the comparison when a baseline
/// exists: a credential change made in this session completes
/// through a state-changing request, where the comparison runs,
/// and one made from another session revokes this session
/// outright — a read-only answer has nothing to catch up on.
async fn reconcile_fingerprint(
    pool: &PgPool,
    session: &Session,
    safe_method: bool,
    token: &str,
) -> String {
    let stored = session
        .get::<String>(SESSION_CSRF_FINGERPRINT)
        .await
        .ok()
        .flatten();
    if stored.is_some() && safe_method {
        return token.to_string();
    }
    match credential_state(pool, session).await {
        Some(CredentialState::Fingerprint(current)) => match stored {
            None => {
                store_fingerprint(session, &current).await;
                token.to_string()
            }
            Some(stored) if stored != current => {
                let rotated = csrf_rotate(session).await;
                store_fingerprint(session, &current).await;
                rotated
            }
            Some(_) => token.to_string(),
        },
        // Withdrawal tears the session down inside the handler:
        // write nothing back into the record it deletes.
        Some(CredentialState::TornDown) => token.to_string(),
        None => {
            tracing::warn!("credential fingerprint unavailable; rotating the CSRF token");
            csrf_rotate(session).await
        }
    }
}

/// Stores the credential fingerprint beside the session's token.
async fn store_fingerprint(session: &Session, fingerprint: &str) {
    let _ = session.insert(SESSION_CSRF_FINGERPRINT, fingerprint).await;
}

/// Writes the token into an HTML answer's forms.
///
/// Only `text/html` responses are touched. A body over the rewrite
/// cap is sent on whole with a warning logged — never truncated
/// into a silent blank page.
async fn inject(res: Response, token: &str) -> Response {
    let is_html = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    if !is_html {
        return res;
    }
    // A body already known to exceed the buffer cap goes out
    // untouched (no tokens injected) instead of being consumed
    // and dropped: the length header is the one signal
    // available before the body is read.
    if res
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|len| len > MAX_BUFFER_BYTES)
    {
        tracing::warn!("HTML response over the CSRF buffer cap; sent without injected tokens");
        return res;
    }
    let (mut parts, body) = res.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BUFFER_BYTES).await else {
        // Unreachable for this app's pages; logged rather than silent.
        tracing::error!("HTML response exceeded the CSRF buffer cap; body dropped");
        return Response::from_parts(parts, Body::empty());
    };
    // The body changes length when rewritten, so a stale length
    // header must go; the rebuilt body reports its own exact size.
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    if bytes.len() > MAX_HTML_BYTES {
        tracing::warn!(
            body_bytes = bytes.len(),
            "HTML response over the CSRF rewrite cap; sent without injected tokens"
        );
        return Response::from_parts(parts, Body::from(bytes));
    }
    Response::from_parts(parts, Body::from(inject_tokens(&bytes, token)))
}

/// Writes the token into an HTML body as a hidden field inside
/// every form that can change state.
///
/// Forms declaring `method="get"` are skipped: their fields travel
/// in the URL, where a token does not belong. htmx forms declare
/// no method and pick the field up through form serialisation.
/// The token is 64 hex characters (see [`crate::session`]), so it
/// needs no HTML escaping.
fn inject_tokens(body: &[u8], token: &str) -> Vec<u8> {
    let field = format!("<input type=\"hidden\" name=\"csrf_token\" value=\"{token}\">");
    let mut out = Vec::with_capacity(body.len() + field.len() * 4);
    let mut rest = body;
    while let Some(pos) = find_form_tag(rest) {
        let tag = &rest[pos..];
        let Some(tag_len) = tag.iter().position(|&b| b == b'>').map(|i| i + 1) else {
            break;
        };
        out.extend_from_slice(&rest[..pos + tag_len]);
        if !is_get_form(&tag[..tag_len]) {
            out.extend_from_slice(field.as_bytes());
        }
        rest = &rest[pos + tag_len..];
    }
    out.extend_from_slice(rest);
    out
}

/// Finds the next genuine `<form` tag opening: the name must be
/// followed by whitespace, `/` or `>`, so longer element names
/// that merely start with "form" never match.
fn find_form_tag(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(5)
        .enumerate()
        .find(|(i, w)| {
            *w == b"<form".as_slice()
                && matches!(
                    bytes.get(i + 5),
                    Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>')
                )
        })
        .map(|(i, _)| i)
}

/// True when a form tag declares the GET method, in any of the
/// spellings HTML allows (`method="get"`, `method='get'`,
/// `method=get`), case-insensitively.
fn is_get_form(tag: &[u8]) -> bool {
    let lower: Vec<u8> = tag.iter().map(|b| b.to_ascii_lowercase()).collect();
    [&b"method=\"get\""[..], b"method='get'", b"method=get"]
        .iter()
        .any(|pat| lower.windows(pat.len()).any(|w| w == *pat))
}

/// Finds the `csrf_token` field in a urlencoded form body.
///
/// Tokens are 64 lowercase hex characters (see [`crate::session`]),
/// which form encoding never escapes, so the value is read
/// literally. `None` when the field is absent.
fn submitted_token(body: &Bytes) -> Option<&str> {
    let text = std::str::from_utf8(body).ok()?;
    text.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "csrf_token").then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "abc123";

    fn field() -> String {
        format!("<input type=\"hidden\" name=\"csrf_token\" value=\"{TOKEN}\">")
    }

    #[test]
    fn token_found_among_other_fields() {
        let body = Bytes::from_static(b"email=a%40b.jp&csrf_token=abc123&next=");
        assert_eq!(submitted_token(&body), Some("abc123"));
    }

    #[test]
    fn token_found_as_only_field() {
        let body = Bytes::from_static(b"csrf_token=deadbeef");
        assert_eq!(submitted_token(&body), Some("deadbeef"));
    }

    #[test]
    fn missing_token_is_none() {
        let body = Bytes::from_static(b"email=a%40b.jp&password=x");
        assert_eq!(submitted_token(&body), None);
    }

    #[test]
    fn similar_field_names_do_not_match() {
        let body = Bytes::from_static(b"csrf_token_extra=abc&x_csrf_token=def");
        assert_eq!(submitted_token(&body), None);
    }

    #[test]
    fn post_form_gets_the_field() {
        let html = format!("<form method=\"post\" action=\"/login\">{}</form>", field());
        assert_eq!(
            inject_tokens(b"<form method=\"post\" action=\"/login\"></form>", TOKEN),
            html.as_bytes()
        );
    }

    #[test]
    fn htmx_form_without_method_gets_the_field() {
        let out = inject_tokens(b"<form hx-post=\"/cart/items\"></form>", TOKEN);
        assert_eq!(
            out,
            format!("<form hx-post=\"/cart/items\">{}</form>", field()).as_bytes()
        );
    }

    #[test]
    fn get_form_is_left_alone() {
        let html = b"<form method=\"get\" action=\"/search\"></form>";
        assert_eq!(inject_tokens(html, TOKEN), html);
    }

    #[test]
    fn every_form_is_served_and_text_between_is_kept() {
        let html = format!(
            "a<form method=\"post\">{}</form>b<form hx-post=\"/x\">{}</form>c",
            field(),
            field()
        );
        assert_eq!(
            inject_tokens(
                b"a<form method=\"post\"></form>b<form hx-post=\"/x\"></form>c",
                TOKEN
            ),
            html.as_bytes()
        );
    }

    #[test]
    fn only_the_standard_safe_methods_skip_the_token() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(is_safe_method(&method));
        }
        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::TRACE,
            Method::CONNECT,
        ] {
            assert!(!is_safe_method(&method));
        }
        // A method nobody has named yet is protected, not safe.
        let novel = Method::from_bytes(b"QUERY").expect("valid method name");
        assert!(!is_safe_method(&novel));
    }

    #[test]
    fn page_without_forms_is_unchanged() {
        let html = b"<p>no forms here</p>";
        assert_eq!(inject_tokens(html, TOKEN), html);
    }
}
