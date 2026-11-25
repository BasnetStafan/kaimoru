//! Menus view (CMS spec C-4).
//!
//! One form for both of the layout's link lists. Every row
//! is a label, a URL, a sort position and two switches —
//! visible and delete; under each menu sit blank rows for
//! adding. Every value is plain text and renders escaped —
//! an operator typing markup into a label sees the literal
//! characters on the storefront, by design (the form says
//! so).

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::{FOOTER_KEY, HEADER_KEY, menu_label};
use crate::services::admin::menu_service::{MenuErrors, MenuItemInput, MenusInput};

/// The menu settings page.
#[derive(Template)]
#[template(path = "admin/pages/content/menus.html")]
pub struct MenusView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The header menu's editor block.
    pub header: MenuBlock,
    /// The footer menu's editor block.
    pub footer: MenuBlock,
    /// One-time form ticket guarding the save.
    pub form_ticket: String,
}

/// One menu's editor block: its stored rows, then the
/// blank add rows, plus the menu-level error (the
/// last-visible-item guard reports against the menu as a
/// whole).
pub struct MenuBlock {
    /// Menu key, used in the form field names.
    pub key: &'static str,
    /// Japanese menu name.
    pub label: &'static str,
    /// The menu-level error.
    pub error: Option<String>,
    /// The stored rows, in submitted order.
    pub rows: Vec<MenuRow>,
    /// The blank add rows.
    pub blank_rows: Vec<MenuRow>,
}

/// One row of the menu editor: the submitted values plus
/// the row's validation messages.
pub struct MenuRow {
    /// Row index within the menu's submission, used in the
    /// form field names and the error lookup.
    pub index: usize,
    /// Stored row id; empty for an add row.
    pub id: String,
    /// Link text as typed.
    pub label: String,
    /// Link URL as typed.
    pub url: String,
    /// Visibility switch.
    pub visible: bool,
    /// Sort position as typed.
    pub sort_order: String,
    /// Delete switch (stored rows only; the template
    /// renders it only there).
    pub delete: bool,
    /// Label error.
    pub label_error: Option<String>,
    /// URL error.
    pub url_error: Option<String>,
    /// Sort position error.
    pub sort_error: Option<String>,
}

/// Builds one editor row from a submitted input row.
fn row(index: usize, input: &MenuItemInput) -> MenuRow {
    MenuRow {
        index,
        id: input.id.clone(),
        label: input.label.clone(),
        url: input.url.clone(),
        visible: input.visible,
        sort_order: input.sort_order.clone(),
        delete: input.delete,
        label_error: None,
        url_error: None,
        sort_error: None,
    }
}

/// Builds one menu's block: rows with an id are the stored
/// rows, rows without one are the add slots (the save
/// ignores untouched ones).
fn block(key: &'static str, rows: &[MenuItemInput]) -> MenuBlock {
    let mut stored = Vec::new();
    let mut blanks = Vec::new();
    for (index, input) in rows.iter().enumerate() {
        if input.id.trim().is_empty() {
            blanks.push(row(index, input));
        } else {
            stored.push(row(index, input));
        }
    }
    MenuBlock {
        key,
        label: menu_label(key),
        error: None,
        rows: stored,
        blank_rows: blanks,
    }
}

impl MenusView {
    /// Builds the page around one set of form values, with
    /// no field errors.
    pub fn new(layout: AdminLayout, values: MenusInput) -> Self {
        Self {
            layout,
            nav: "content_menus",
            header: block(HEADER_KEY, &values.header),
            footer: block(FOOTER_KEY, &values.footer),
            form_ticket: String::new(),
        }
    }

    /// Pins a validation failure onto the page: the
    /// messages sit parallel to the submitted rows, so each
    /// lands on its own row by index.
    pub fn apply_errors(&mut self, errors: MenuErrors) {
        self.header.error = errors.header_menu;
        self.footer.error = errors.footer_menu;
        for block in [&mut self.header, &mut self.footer] {
            let rows = if block.key == HEADER_KEY {
                &errors.header_rows
            } else {
                &errors.footer_rows
            };
            for row in block.rows.iter_mut().chain(block.blank_rows.iter_mut()) {
                if let Some(messages) = rows.get(row.index) {
                    row.label_error = messages.label.clone();
                    row.url_error = messages.url.clone();
                    row.sort_error = messages.sort.clone();
                }
            }
        }
    }
}
