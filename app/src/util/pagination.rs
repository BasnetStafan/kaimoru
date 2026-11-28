//! Pagination arithmetic shared by every surface: one
//! definition of the page-count rounding rule, so the admin
//! lists, the storefront search and the member history can
//! never drift apart on what a partial page counts as.

/// Returns how many pages `total` rows make at `per_page` rows
/// a page, rounding a partial last page up. Zero rows make zero
/// pages; a caller that must still show a pager for an empty
/// list clamps the result to at least 1.
pub fn total_pages(total: i64, per_page: i64) -> i64 {
    (total + per_page - 1) / per_page
}
