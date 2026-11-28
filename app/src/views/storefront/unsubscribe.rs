//! Unsubscribe views: the no-login 配信停止
//! confirmation and completion pages.

use askama::Template;

use crate::views::storefront::header::Header;

/// The unsubscribe confirmation page: the token rides in
/// the form action, nothing else identifies the member.
#[derive(Template)]
#[template(path = "storefront/pages/unsubscribe/confirm.html")]
pub struct UnsubscribeConfirmView {
    /// Shared storefront header state.
    pub header: Header,
    /// The unsubscribe token from the mail link.
    pub token: String,
}

/// The unsubscribe completion page.
#[derive(Template)]
#[template(path = "storefront/pages/unsubscribe/done.html")]
pub struct UnsubscribeDoneView {
    /// Shared storefront header state.
    pub header: Header,
    /// Whether the member was already unsubscribed when
    /// the page was opened (wording only).
    pub already: bool,
}

/// The invalid-link page: an unknown token gets a
/// dedicated explanation instead
/// of the generic 404 — the visitor followed a mail link
/// and deserves to know the link is the problem and what
/// to do instead. Every unknown token renders this same
/// page, so it is no oracle for any other token.
#[derive(Template)]
#[template(path = "storefront/pages/unsubscribe/invalid.html")]
pub struct UnsubscribeInvalidView {
    /// Shared storefront header state.
    pub header: Header,
}
