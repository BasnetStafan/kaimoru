//! Home content view (CMS spec C-3).
//!
//! One form for the whole top page: the announcement bar,
//! the hero, and the section rows. Every value is plain
//! text and renders escaped — an operator typing markup
//! into a field sees the literal characters on the
//! storefront, by design (the form says so).

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::{ProductOption, section_label};
use crate::services::admin::home_content_service::{HomeContentInput, PinEditorData};
use crate::services::product_service;

/// The top page settings page.
#[derive(Template)]
#[template(path = "admin/pages/content/home.html")]
pub struct HomeContentView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The form values: the stored content, or the submitted
    /// one when validation failed (so the operator fixes it
    /// instead of retyping).
    pub values: HomeContentInput,
    /// The section rows, in canonical order.
    pub sections: Vec<SectionRow>,
    /// Announcement text error.
    pub announcement_text_error: Option<String>,
    /// Announcement link label error.
    pub announcement_link_label_error: Option<String>,
    /// Announcement link URL error.
    pub announcement_link_url_error: Option<String>,
    /// Announcement window start error.
    pub announcement_starts_at_error: Option<String>,
    /// Announcement window end error.
    pub announcement_ends_at_error: Option<String>,
    /// Hero headline error.
    pub hero_headline_error: Option<String>,
    /// Hero subtext error.
    pub hero_text_error: Option<String>,
    /// Hero image error.
    pub hero_images_error: Option<String>,
    /// Hero window start error.
    pub hero_starts_at_error: Option<String>,
    /// Hero window end error.
    pub hero_ends_at_error: Option<String>,
    /// The pin editors, one per pinnable section.
    pub pin_editors: Vec<PinEditorRow>,
    /// The products the pin add-pickers offer.
    pub pin_options: Vec<PinOptionRow>,
    /// One-time form ticket guarding the save.
    pub form_ticket: String,
}

/// One section's pin editor: its current pins in order and
/// the add-picker below them.
pub struct PinEditorRow {
    /// Section key, used in the form field names.
    pub key: String,
    /// Japanese section name.
    pub label: String,
    /// How the section fills around its pins.
    pub note: String,
    /// The pin cap: the section's row size, or 0 for the
    /// featured section, which takes pins without a cap.
    pub cap: i64,
    /// The current pins, in pin order.
    pub pins: Vec<PinRowView>,
    /// The pin ids comma-joined, the editor's hidden field
    /// value (the script keeps it in step with the rows).
    pub joined_ids: String,
    /// Pin list error.
    pub error: Option<String>,
}

/// One pin row in the editor.
pub struct PinRowView {
    /// The pinned product.
    pub id: i64,
    /// The pinned product's name.
    pub name: String,
    /// Status suffix for a non-public product; empty for a
    /// public one.
    pub status_label: String,
}

/// One product in the pin add-picker.
pub struct PinOptionRow {
    /// Product id.
    pub id: i64,
    /// Name with a status suffix for non-public products.
    pub label: String,
}

/// The status suffix the pin editor shows for a product:
/// a public product needs none.
fn status_label(status: &str) -> String {
    match status {
        "public" => String::new(),
        "draft" => "（下書き）".to_string(),
        _ => "（非公開）".to_string(),
    }
}

/// A pin section's editor label and fill note.
/// Returns where a section's description text shows, for
/// the note under its editor input.
fn description_hint(key: &str) -> &'static str {
    match key {
        "featured" => {
            "トップページの特集バナーと、特集ページ（/special）の両方に表示されます。空欄にすると初期文に戻ります。"
        }
        "free_space" => {
            "トップページのこのセクションの本文として表示されます。空欄にすると初期文に戻ります。"
        }
        _ => "空欄にすると初期文に戻ります。",
    }
}

fn pin_label_note(key: &str) -> (String, String) {
    match key {
        "new_arrivals" => (
            "新着商品".to_string(),
            "先頭に固定表示され、残りの枠は新着順で埋まります。".to_string(),
        ),
        "ranking" => (
            "ランキング".to_string(),
            "先頭に固定表示され、残りの枠は売上順で埋まります。".to_string(),
        ),
        "event" => (
            "イベント受取商品".to_string(),
            "イベント商品だけを固定できます。先頭に固定表示され、残りの枠は新着順で埋まります。"
                .to_string(),
        ),
        "featured" => (
            "特集".to_string(),
            format!(
                "固定した商品が特集になります（トップページでは先頭から{}件、特集ページでは全件）。件数の上限はありません。固定がない場合は、これまでどおりバナーが表示されます。",
                product_service::section_limit("featured").unwrap_or(0)
            ),
        ),
        _ => (String::new(), String::new()),
    }
}

/// One section's editor row: its fixed name, the built-in
/// heading (shown as the input's placeholder) and the
/// operator's override, switch and sort position.
pub struct SectionRow {
    /// Section key, used in the form field names.
    pub key: String,
    /// Japanese section name.
    pub label: &'static str,
    /// The built-in heading, the override input's
    /// placeholder.
    pub default_heading: &'static str,
    /// Heading override as typed.
    pub heading: String,
    /// Description override as typed.
    pub description: String,
    /// Whether this section renders a description field:
    /// only sections whose templates carry a body text
    /// (the featured banner/page, the free-space strip)
    /// offer the input.
    pub has_description: bool,
    /// The description input's placeholder: the section's
    /// own built-in sentence.
    pub description_placeholder: &'static str,
    /// Where the description shows, said under the input;
    /// per section, since the featured text and the strip
    /// text live in different places.
    pub description_hint: &'static str,
    /// Visibility switch.
    pub visible: bool,
    /// Sort position as typed.
    pub sort_order: String,
    /// Heading field error.
    pub heading_error: Option<String>,
    /// Description field error.
    pub description_error: Option<String>,
    /// Sort field error.
    pub sort_error: Option<String>,
}

impl HomeContentView {
    /// Builds the page around one set of form values, with
    /// no field errors.
    pub fn new(
        layout: AdminLayout,
        values: HomeContentInput,
        editors: Vec<PinEditorData>,
        options: Vec<ProductOption>,
    ) -> Self {
        let sections = values
            .sections
            .iter()
            .map(|row| SectionRow {
                key: row.key.clone(),
                label: section_label(&row.key),
                default_heading: crate::models::default_heading(&row.key),
                heading: row.heading.clone(),
                description: row.description.clone(),
                has_description: row.key == "featured" || row.key == "free_space",
                description_placeholder: crate::models::default_description(&row.key),
                description_hint: description_hint(&row.key),
                visible: row.visible,
                sort_order: row.sort_order.clone(),
                heading_error: None,
                description_error: None,
                sort_error: None,
            })
            .collect();
        let pin_editors = editors
            .into_iter()
            .map(|editor| {
                let (label, note) = pin_label_note(&editor.key);
                let pins: Vec<PinRowView> = editor
                    .rows
                    .into_iter()
                    .map(|row| PinRowView {
                        id: row.product_id,
                        name: row.name,
                        status_label: status_label(&row.status),
                    })
                    .collect();
                let joined_ids = pins
                    .iter()
                    .map(|pin| pin.id.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                PinEditorRow {
                    cap: product_service::pin_cap(&editor.key).unwrap_or(0),
                    label,
                    note,
                    pins,
                    joined_ids,
                    error: None,
                    key: editor.key,
                }
            })
            .collect();
        let pin_options = options
            .into_iter()
            .map(|option| PinOptionRow {
                id: option.id,
                label: format!("{}{}", option.name, status_label(&option.status)),
            })
            .collect();
        Self {
            layout,
            nav: "content_home",
            values,
            sections,
            pin_editors,
            pin_options,
            announcement_text_error: None,
            announcement_link_label_error: None,
            announcement_link_url_error: None,
            announcement_starts_at_error: None,
            announcement_ends_at_error: None,
            hero_headline_error: None,
            hero_text_error: None,
            hero_images_error: None,
            hero_starts_at_error: None,
            hero_ends_at_error: None,
            form_ticket: String::new(),
        }
    }
}
