//! Admin permission guard.
//!
//! Sits inside the IP allowlist guard and the admin session
//! layer: by the time a request reaches this layer, its
//! session exists, so the signed-in admin resolves exactly
//! the way the handlers resolve it ([`crate::session::current_admin`],
//! fresh from the database — a permission change or a
//! deactivation applies on the very next request).
//!
//! A staff admin who asks for an area outside their granted
//! permissions gets a styled 403. Unauthenticated requests
//! pass through untouched: the handlers' `CurrentAdmin`
//! extractor owns the redirect to the login page, and the
//! login page itself stays reachable because `/admin/login`
//! maps to no area.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::models::{
    AREA_CONTENT, AREA_MAIL, AREA_PRODUCTS, AREA_SALES, AREA_SETTINGS, AREA_STORE,
};
use crate::session::current_admin;

/// What a path under `/admin` requires of the signed-in admin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    /// Any active admin: the dashboard, the admin's own
    /// account settings, and the login/logout posts — the
    /// explicitly listed paths only.
    Any,
    /// One permission area (an `AREA_*` key).
    Area(&'static str),
    /// Owners only: the admin-account management screens.
    OwnerOnly,
}

/// Maps a request path to its gate. Prefixes match on segment
/// boundaries: `/admin/mail` covers `/admin/mail/templates`
/// but not a hypothetical `/admin/mailbox`.
fn gate_for(path: &str) -> Gate {
    // The paths every signed-in admin may use, listed
    // explicitly: with the fallthrough now OwnerOnly
    //, anything not named here or in
    // the table below is owner-only by default, so a route
    // added under a new prefix ships closed, not open.
    // The dashboard is an exact path (as a prefix it
    // would swallow everything); the account paths match
    // as prefixes on segment boundaries, like the table —
    // the profile area carries sub-pages of its own (the
    // TOTP setup among them).
    if path == "/admin" {
        return Gate::Any;
    }
    const ANY_PREFIXES: [&str; 4] = [
        "/admin/login",
        "/admin/logout",
        "/admin/profile",
        "/admin/password",
    ];
    for prefix in ANY_PREFIXES {
        if let Some(rest) = path.strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return Gate::Any;
        }
    }
    const TABLE: [(&str, Gate); 18] = [
        ("/admin/admins", Gate::OwnerOnly),
        ("/admin/orders", Gate::Area(AREA_SALES)),
        ("/admin/analytics", Gate::Area(AREA_SALES)),
        ("/admin/returns", Gate::Area(AREA_SALES)),
        ("/admin/segments", Gate::Area(AREA_STORE)),
        ("/admin/payments", Gate::Area(AREA_SALES)),
        ("/admin/inquiries", Gate::Area(AREA_SALES)),
        ("/admin/reviews", Gate::Area(AREA_SALES)),
        ("/admin/products", Gate::Area(AREA_PRODUCTS)),
        ("/admin/categories", Gate::Area(AREA_PRODUCTS)),
        ("/admin/tags", Gate::Area(AREA_PRODUCTS)),
        ("/admin/notices", Gate::Area(AREA_PRODUCTS)),
        ("/admin/content", Gate::Area(AREA_CONTENT)),
        ("/admin/coupons", Gate::Area(AREA_CONTENT)),
        ("/admin/mail", Gate::Area(AREA_MAIL)),
        ("/admin/customers", Gate::Area(AREA_STORE)),
        ("/admin/audit", Gate::Area(AREA_STORE)),
        ("/admin/settings", Gate::Area(AREA_SETTINGS)),
    ];
    for (prefix, gate) in TABLE {
        if let Some(rest) = path.strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return gate;
        }
    }
    // Deny by default: an unlisted path (a new area whose
    // registration forgot this table, or a path that does
    // not exist at all) is owner-only. A staff admin
    // probing a mistyped URL sees the styled 403 instead
    // of the 404 — a fair trade for never shipping a new
    // area open to every staff account.
    Gate::OwnerOnly
}

/// The admin router's permission middleware: see the module
/// docs. Only a KNOWN admin is ever refused here; anything
/// else passes through to the handlers and their extractor.
pub async fn require_permission(State(pool): State<PgPool>, req: Request, next: Next) -> Response {
    let gate = gate_for(req.uri().path());
    if gate == Gate::Any {
        return next.run(req).await;
    }
    let Some(session) = req.extensions().get::<Session>().cloned() else {
        return next.run(req).await;
    };
    let Some(admin) = current_admin(&pool, &session).await else {
        return next.run(req).await;
    };
    let allowed = match gate {
        Gate::Any => true,
        Gate::Area(area) => admin.allows(area),
        Gate::OwnerOnly => admin.is_owner(),
    };
    if allowed {
        next.run(req).await
    } else {
        // IDs and the path only — never who holds what.
        tracing::warn!(
            admin_id = admin.id,
            path = req.uri().path(),
            "admin refused: missing permission"
        );
        (
            StatusCode::FORBIDDEN,
            Html(crate::views::admin::notfound::bare_403_html()),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{Gate, gate_for};
    use crate::models::{AREA_MAIL, AREA_SETTINGS};

    #[test]
    fn dashboard_and_account_paths_are_any_admin() {
        assert_eq!(gate_for("/admin"), Gate::Any);
        assert_eq!(gate_for("/admin/login"), Gate::Any);
        assert_eq!(gate_for("/admin/logout"), Gate::Any);
        assert_eq!(gate_for("/admin/profile"), Gate::Any);
        assert_eq!(gate_for("/admin/password"), Gate::Any);
    }

    #[test]
    fn unlisted_paths_are_owner_only_by_default() {
        assert_eq!(gate_for("/admin/no-such-page"), Gate::OwnerOnly);
        assert_eq!(gate_for("/admin/future-area"), Gate::OwnerOnly);
    }

    #[test]
    fn area_prefixes_cover_their_subpaths() {
        assert_eq!(gate_for("/admin/mail"), Gate::Area(AREA_MAIL));
        assert_eq!(gate_for("/admin/mail/templates"), Gate::Area(AREA_MAIL));
        assert_eq!(gate_for("/admin/settings/shop"), Gate::Area(AREA_SETTINGS));
    }

    #[test]
    fn prefixes_stop_at_segment_boundaries() {
        // Not the mail/settings areas — and with the
        // deny-by-default fallthrough, owner-only.
        assert_eq!(gate_for("/admin/mailbox"), Gate::OwnerOnly);
        assert_eq!(gate_for("/admin/settings2"), Gate::OwnerOnly);
    }

    #[test]
    fn account_management_is_owner_only() {
        assert_eq!(gate_for("/admin/admins"), Gate::OwnerOnly);
        assert_eq!(gate_for("/admin/admins/3/edit"), Gate::OwnerOnly);
    }
}
