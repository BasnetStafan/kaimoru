//! Request logging for Kaimoru.
//! One middleware logs every end user request as a single structured line,
//! so actions in the store can be followed and problems can be traced.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};
use tower_sessions::Session;
use uuid::Uuid;

use crate::session::{SESSION_ADMIN_ID, SESSION_USER_ID};

// Privacy rules, enforced here, in the middleware every page,
// admin and webhook request passes:
//   - Path only, never the full URL: query strings can carry tokens
//     or search terms.
//   - Never headers (they carry credentials) and never bodies (they
//     carry names, addresses, order details).
//   - Requests are identified by a random request ID, never by IP
//     or cookie.
//   - Two deliberate exceptions let the operator follow one visitor
//     while debugging: the member's user_id (an internal surrogate,
//     meaningless without database access) and a session fingerprint
//     (the first 12 hex characters of the SHA-256 of the session
//     ID). The raw session ID is a live credential and is never
//     logged; the fingerprint groups one session's lines without
//     being usable or reversible, and changes when the ID is
//     regenerated (login, sign-up), after which user_id is the
//     stable key.
//   - Admin lines follow the same rule with admin_id in its own
//     field, so `user` keeps one meaning on every line.
//   - Every line carries the surface that served it — not personal
//     data, and what routes the line to that surface's log file.
// Business actions (cart changes, checkout) follow the same rules:
// action name and record IDs only.

/// The surface a request belongs to, as recorded in the request log.
///
/// Each router assembly registers its own variant (see
/// [`crate::routes`]), so the logged value is declared by how the
/// route was assembled, never guessed from the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The member-facing storefront.
    Storefront,
    /// The `/admin` back office.
    Admin,
    /// The provider webhook receiver: no session stack, so its
    /// lines read as guest with no fingerprint.
    Webhook,
    /// The agent-readable API: no session stack, so its lines
    /// read as guest with no fingerprint.
    Api,
}

impl Surface {
    /// The value written to the log line's `surface` field.
    fn as_log_value(&self) -> &'static str {
        match self {
            Surface::Storefront => "storefront",
            Surface::Admin => "admin",
            Surface::Webhook => "webhook",
            Surface::Api => "api",
        }
    }
}

/// Logs one structured line for a single request.
///
/// The line carries request ID, surface, method, path, status and
/// latency — plus `admin` on the admin surface — under the privacy
/// rules above. The request ID is also returned in the
/// `x-request-id` header, so a report matches exactly one line.
pub async fn request_log(State(surface): State<Surface>, req: Request, next: Next) -> Response {
    use tracing::Instrument as _;

    let request_id = Uuid::new_v4();
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let started = Instant::now();

    // The session layer sits outside this middleware, so the handle
    // is already in the extensions; cloned so it shows the final
    // state after the handler runs (a login included).
    let session = req.extensions().get::<Session>().cloned();

    // The span records the surface so every line the handler
    // emits can be routed to that surface's own log file.
    let span = tracing::info_span!("request", surface = %surface.as_log_value());
    let mut response = next.run(req).instrument(span).await;

    // Identity for debugging, under the file-top rules: member
    // number or "guest"; the session only as a fingerprint, or "-"
    // on a new session's first request (its ID is assigned when the
    // session layer saves it, after this line is written).
    let (user, admin, session_fp) = match &session {
        Some(session) => {
            // `user` keeps one meaning on every surface: the member
            // number, or "guest". The admin session carries no member
            // number, so the admin goes in his own field instead.
            let user = session
                .get::<i64>(SESSION_USER_ID)
                .await
                .ok()
                .flatten()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "guest".to_string());
            // The admin number, under the same surrogate-ID rule;
            // "-" before login. Only the admin surface reads it.
            let admin = match surface {
                Surface::Admin => session
                    .get::<i64>(SESSION_ADMIN_ID)
                    .await
                    .ok()
                    .flatten()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                Surface::Storefront | Surface::Webhook | Surface::Api => "-".to_string(),
            };
            let fp = session
                .id()
                .map(|id| session_fingerprint(&id.to_string()))
                .unwrap_or_else(|| "-".to_string());
            (user, admin, fp)
        }
        None => ("guest".to_string(), "-".to_string(), "-".to_string()),
    };

    if let Ok(value) = request_id.to_string().parse() {
        response.headers_mut().insert("x-request-id", value);
    }

    // Each surface has its own log file; the subscriber routes
    // on this `surface` field. `admin` is emitted on admin lines
    // only, so storefront lines keep their established shape.
    match surface {
        Surface::Admin => tracing::info!(
            request_id = %request_id,
            surface = %surface.as_log_value(),
            user = %user,
            admin = %admin,
            session_fp = %session_fp,
            method = %method,
            path = %path,
            status = %response.status(),
            latency_ms = started.elapsed().as_millis(),
            "request completed"
        ),
        Surface::Storefront | Surface::Webhook | Surface::Api => tracing::info!(
            request_id = %request_id,
            surface = %surface.as_log_value(),
            user = %user,
            session_fp = %session_fp,
            method = %method,
            path = %path,
            status = %response.status(),
            latency_ms = started.elapsed().as_millis(),
            "request completed"
        ),
    }

    response
}

/// Truncates a session ID to a correlation fingerprint: the
/// first 12 hex characters of its SHA-256. Enough to group one
/// session's log lines, useless for reconstructing the ID.
pub(crate) fn session_fingerprint(session_id: &str) -> String {
    let digest = Sha256::digest(session_id.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
