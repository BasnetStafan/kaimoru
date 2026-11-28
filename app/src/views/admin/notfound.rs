//! The styled admin 404, and the fixed standalone error
//! documents (403, 500) rendered without a session.
//!
//! Rendered by the admin router's fallback for unknown paths
//! and by controllers for records that do not exist — the two
//! are indistinguishable on purpose, so the page never
//! confirms which admin URLs or record IDs are real.

use askama::Template;

use crate::controllers::admin::AdminLayout;

/// The admin "page not found" page, inside the admin layout
/// when an operator is signed in. The router fallback has no
/// signed-in admin, so it renders [`bare_404_html`] instead.
#[derive(Template)]
#[template(path = "admin/pages/notfound.html")]
pub struct NotFoundView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key (none highlighted on a 404).
    pub nav: &'static str,
}

/// The same 404 as a complete standalone document, for the
/// router fallback where no admin session exists. Kept minimal
/// markup in code — it is one fixed page, not a template with
/// data. (The controllers' render path always has an admin and
/// uses the templated version.) The stylesheet
/// URLs carry the shared asset fingerprint, like every template's.
pub fn bare_404_html() -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"ja\">\n<head>\n<meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     <title>ページが見つかりません | AGENTIC STORE 管理</title>\n\
     <link rel=\"stylesheet\" href=\"/static/css/style.css?v={}\">\n\
     <link rel=\"stylesheet\" href=\"/static/css/admin.css?v={}\">\n</head>\n\
     <body class=\"admin-body\">\n<main class=\"admin-bare\">\n\
     <div class=\"admin-card admin-notfound\">\n\
     <h1>ページが見つかりません</h1>\n\
     <p>URLが間違っているか、ページが移動・削除された可能性があります。</p>\n\
     <p><a href=\"/admin\">ダッシュボードへ戻る</a></p>\n\
     </div>\n</main>\n</body>\n</html>",
        crate::util::asset_fingerprint::asset_fingerprint(),
        crate::util::asset_fingerprint::asset_fingerprint()
    )
}

/// The styled 500 as a complete standalone document
/// the admin render tail and the
/// DB-error tail answer with this page instead of a plain
/// English "Internal Server Error". Standalone like the
/// bare 404/403 — a render failure may ride on a broken
/// layout path, so the page depends on nothing dynamic
/// beyond the asset fingerprint.
pub fn bare_500_html() -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"ja\">\n<head>\n<meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     <title>システムエラー | AGENTIC STORE 管理</title>\n\
     <link rel=\"stylesheet\" href=\"/static/css/style.css?v={}\">\n\
     <link rel=\"stylesheet\" href=\"/static/css/admin.css?v={}\">\n</head>\n\
     <body class=\"admin-body\">\n<main class=\"admin-bare\">\n\
     <div class=\"admin-card admin-notfound\">\n\
     <h1>システムエラーが発生しました</h1>\n\
     <p>一時的な問題が発生しています。時間をおいて、もう一度お試しください。</p>\n\
     <p><a href=\"/admin\">ダッシュボードへ戻る</a></p>\n\
     </div>\n</main>\n</body>\n</html>",
        crate::util::asset_fingerprint::asset_fingerprint(),
        crate::util::asset_fingerprint::asset_fingerprint()
    )
}

/// The styled 403 as a complete standalone document, for the
/// permission guard: the requesting admin IS signed in, but a
/// page inside the admin layout would promise navigation the
/// admin may not use, so — like the bare 404 — this stays a
/// minimal fixed page whose only link is the dashboard (every
/// active admin may see it).
pub fn bare_403_html() -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"ja\">\n<head>\n<meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     <title>権限がありません | AGENTIC STORE 管理</title>\n\
     <link rel=\"stylesheet\" href=\"/static/css/style.css?v={}\">\n\
     <link rel=\"stylesheet\" href=\"/static/css/admin.css?v={}\">\n</head>\n\
     <body class=\"admin-body\">\n<main class=\"admin-bare\">\n\
     <div class=\"admin-card admin-notfound\">\n\
     <h1>権限がありません</h1>\n\
     <p>このページを表示する権限がありません。必要な場合はオーナー管理者にお問い合わせください。</p>\n\
     <p><a href=\"/admin\">ダッシュボードへ戻る</a></p>\n\
     </div>\n</main>\n</body>\n</html>",
        crate::util::asset_fingerprint::asset_fingerprint(),
        crate::util::asset_fingerprint::asset_fingerprint()
    )
}
