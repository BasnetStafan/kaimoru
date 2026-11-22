//! Home content (CMS spec C-3).
//!
//! The operator's editor for the top page's words: the
//! announcement bar, the hero, and the sections' headings,
//! visibility and order. The rules live in
//! `crate::services::admin::home_content_service` — lengths,
//! the link and window rules, and the single audited write.
//! Handlers here own HTTP concerns only.
//!
//! Success answers with a redirect back to this page (the
//! admin surface's Post/Redirect/Get pattern).

use axum::{
    Form, Router,
    extract::State,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, render, set_flash};
use crate::services::admin::home_content_service::{
    self, HomeContentInput, HomeSectionInput, PinInput,
};
use crate::session::CurrentAdmin;
use crate::validation::{FieldError, field_error};
use crate::views::admin::home_content::HomeContentView;

/// Routes for the home content surface.
pub fn router() -> Router<PgPool> {
    Router::new().route("/admin/content/home", get(page).post(update))
}

/// The home content form body: every field a string,
/// exactly as typed — trimming and validation live in the
/// service. Checkboxes arrive as a value when checked and
/// not at all when clear. The section fields are spelled
/// per key because the key set is fixed in code.
#[derive(Deserialize)]
pub struct HomeContentForm {
    /// One-time form ticket from the editor render.
    #[serde(default)]
    form_ticket: String,
    /// Announcement switch.
    #[serde(default)]
    announcement_enabled: Option<String>,
    /// Announcement text.
    #[serde(default)]
    announcement_text: String,
    /// Announcement link label.
    #[serde(default)]
    announcement_link_label: String,
    /// Announcement link URL.
    #[serde(default)]
    announcement_link_url: String,
    /// Announcement window start.
    #[serde(default)]
    announcement_starts_at: String,
    /// Announcement window end.
    #[serde(default)]
    announcement_ends_at: String,
    /// Hero switch.
    #[serde(default)]
    hero_enabled: Option<String>,
    /// Hero headline.
    #[serde(default)]
    hero_headline: String,
    /// Hero subtext.
    #[serde(default)]
    hero_text: String,
    /// Hero image paths, comma-joined in display order
    /// (the editor's one hidden field; the form parser
    /// rejects repeated keys).
    #[serde(default)]
    hero_images: String,
    /// Hero slide links, comma-joined in the same order as
    /// `hero_images` (an unlinked slide's entry is empty).
    #[serde(default)]
    hero_image_links: String,
    /// Hero window start.
    #[serde(default)]
    hero_starts_at: String,
    /// Hero window end.
    #[serde(default)]
    hero_ends_at: String,
    /// Categories heading override.
    #[serde(default)]
    heading_categories: String,
    /// Categories visibility.
    #[serde(default)]
    visible_categories: Option<String>,
    /// Categories sort position.
    #[serde(default)]
    sort_categories: String,
    /// New arrivals heading override.
    #[serde(default)]
    heading_new_arrivals: String,
    /// New arrivals visibility.
    #[serde(default)]
    visible_new_arrivals: Option<String>,
    /// New arrivals sort position.
    #[serde(default)]
    sort_new_arrivals: String,
    /// Ranking heading override.
    #[serde(default)]
    heading_ranking: String,
    /// Ranking visibility.
    #[serde(default)]
    visible_ranking: Option<String>,
    /// Ranking sort position.
    #[serde(default)]
    sort_ranking: String,
    /// Featured heading override.
    #[serde(default)]
    heading_featured: String,
    /// Featured visibility.
    #[serde(default)]
    visible_featured: Option<String>,
    /// Featured sort position.
    #[serde(default)]
    sort_featured: String,
    /// Notices heading override.
    #[serde(default)]
    heading_notices: String,
    /// Notices visibility.
    #[serde(default)]
    visible_notices: Option<String>,
    /// Notices sort position.
    #[serde(default)]
    sort_notices: String,
    /// Staff picks heading override.
    #[serde(default)]
    heading_staff_picks: String,
    /// Staff picks visibility.
    #[serde(default)]
    visible_staff_picks: Option<String>,
    /// Staff picks sort position.
    #[serde(default)]
    sort_staff_picks: String,
    /// Free-space heading override.
    #[serde(default)]
    heading_free_space: String,
    /// Free-space visibility.
    #[serde(default)]
    visible_free_space: Option<String>,
    /// Free-space sort position.
    #[serde(default)]
    sort_free_space: String,
    /// Free-space description override.
    #[serde(default)]
    description_free_space: String,
    /// Categories description override.
    #[serde(default)]
    description_categories: String,
    /// New arrivals description override.
    #[serde(default)]
    description_new_arrivals: String,
    /// Ranking description override.
    #[serde(default)]
    description_ranking: String,
    /// Featured description override.
    #[serde(default)]
    description_featured: String,
    /// Staff picks description override.
    #[serde(default)]
    description_staff_picks: String,
    /// Notices description override.
    #[serde(default)]
    description_notices: String,
    /// New arrivals pin ids, comma-joined in display
    /// order (the editor's one hidden field per section).
    #[serde(default)]
    pins_new_arrivals: Option<String>,
    /// Ranking pin ids, comma-joined.
    #[serde(default)]
    pins_ranking: Option<String>,
    /// Event pin ids, comma-joined.
    #[serde(default)]
    pins_event: Option<String>,
    /// Featured pin ids, comma-joined.
    #[serde(default)]
    pins_featured: Option<String>,
}

/// Builds one pin section's input from its comma-joined
/// field: a section whose field is absent from the submit
/// is absent here too (its stored pins are kept — a partial
/// submit must never clear what it did not show), and
/// fragments that do not parse as ids are dropped, since
/// the field is machine-generated and an unparseable
/// fragment can only be tampering.
fn pin_input(key: &str, raw: Option<String>) -> Option<PinInput> {
    let raw = raw?;
    let product_ids = raw
        .split(',')
        .filter_map(|value| value.trim().parse::<i64>().ok())
        .collect();
    Some(PinInput {
        key: key.to_string(),
        product_ids,
    })
}

/// Builds one section's input row from its form fields.
fn section_input(
    key: &str,
    heading: String,
    description: String,
    visible: Option<String>,
    sort_order: String,
) -> HomeSectionInput {
    HomeSectionInput {
        key: key.to_string(),
        heading,
        description,
        visible: visible.is_some(),
        sort_order,
    }
}

impl From<HomeContentForm> for HomeContentInput {
    /// Carries the submitted form into the service's input
    /// shape: the two slots' fields verbatim and the section
    /// fields zipped into rows, in canonical key order.
    fn from(form: HomeContentForm) -> Self {
        Self {
            announcement_enabled: form.announcement_enabled.is_some(),
            announcement_text: form.announcement_text,
            announcement_link_label: form.announcement_link_label,
            announcement_link_url: form.announcement_link_url,
            announcement_starts_at: form.announcement_starts_at,
            announcement_ends_at: form.announcement_ends_at,
            hero_enabled: form.hero_enabled.is_some(),
            hero_headline: form.hero_headline,
            hero_text: form.hero_text,
            hero_images: form
                .hero_images
                .split(',')
                .map(|path| path.trim().to_string())
                .filter(|path| !path.is_empty())
                .zip(
                    form.hero_image_links
                        .split(',')
                        .map(|url| url.trim().to_string())
                        .chain(std::iter::repeat_with(String::new)),
                )
                .map(|(image_path, link_url)| crate::models::HeroImage {
                    image_path,
                    link_url,
                })
                .collect(),
            hero_starts_at: form.hero_starts_at,
            hero_ends_at: form.hero_ends_at,
            sections: vec![
                section_input(
                    "categories",
                    form.heading_categories,
                    form.description_categories,
                    form.visible_categories,
                    form.sort_categories,
                ),
                section_input(
                    "new_arrivals",
                    form.heading_new_arrivals,
                    form.description_new_arrivals,
                    form.visible_new_arrivals,
                    form.sort_new_arrivals,
                ),
                section_input(
                    "free_space",
                    form.heading_free_space,
                    form.description_free_space,
                    form.visible_free_space,
                    form.sort_free_space,
                ),
                section_input(
                    "ranking",
                    form.heading_ranking,
                    form.description_ranking,
                    form.visible_ranking,
                    form.sort_ranking,
                ),
                section_input(
                    "featured",
                    form.heading_featured,
                    form.description_featured,
                    form.visible_featured,
                    form.sort_featured,
                ),
                section_input(
                    "staff_picks",
                    form.heading_staff_picks,
                    form.description_staff_picks,
                    form.visible_staff_picks,
                    form.sort_staff_picks,
                ),
                section_input(
                    "notices",
                    form.heading_notices,
                    form.description_notices,
                    form.visible_notices,
                    form.sort_notices,
                ),
            ],
            pins: [
                pin_input("new_arrivals", form.pins_new_arrivals),
                pin_input("ranking", form.pins_ranking),
                pin_input("event", form.pins_event),
                pin_input("featured", form.pins_featured),
            ]
            .into_iter()
            .flatten()
            .collect(),
        }
    }
}

/// GET /admin/content/home: the editor, freshly rendered
/// with the stored values.
pub async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    match home_content_service::get_home(&pool).await {
        Ok(content) => {
            form_view(
                &pool,
                &admin,
                &session,
                HomeContentInput::from(&content),
                &[],
            )
            .await
        }
        Err(e) => db_error("home content", e),
    }
}

/// Renders the editor around one set of form values, with
/// the given field errors applied (slot, section and pin
/// errors alike). The pin editors resolve against the
/// values, so a refused submit shows the pins as submitted.
async fn form_view(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    input: HomeContentInput,
    errors: &[FieldError],
) -> Response {
    let (editors, options) = match home_content_service::pin_editor_data(pool, &input).await {
        Ok(data) => data,
        Err(e) => return db_error("home content pins", e),
    };
    let stored = match home_content_service::get_home(pool).await {
        Ok(content) => content,
        Err(e) => return db_error("home content", e),
    };
    let mut view = HomeContentView::new(
        layout_for(pool, admin, session).await,
        input,
        editors,
        options,
    );
    view.announcement_text_error = field_error(errors, "announcement_text");
    view.announcement_link_label_error = field_error(errors, "announcement_link_label");
    view.announcement_link_url_error = field_error(errors, "announcement_link_url");
    view.announcement_starts_at_error = field_error(errors, "announcement_starts_at");
    view.announcement_ends_at_error = field_error(errors, "announcement_ends_at");
    view.hero_headline_error = field_error(errors, "hero_headline");
    view.hero_text_error = field_error(errors, "hero_text");
    view.hero_images_error = field_error(errors, "hero_images");
    view.hero_starts_at_error = field_error(errors, "hero_starts_at");
    view.hero_ends_at_error = field_error(errors, "hero_ends_at");
    for row in view.sections.iter_mut() {
        row.heading_error = field_error(errors, &format!("section_heading_{}", row.key));
        row.description_error = field_error(errors, &format!("section_description_{}", row.key));
        row.sort_error = field_error(errors, &format!("section_sort_{}", row.key));
    }
    for editor in view.pin_editors.iter_mut() {
        editor.error = field_error(errors, &format!("pins_{}", editor.key));
    }
    view.form_ticket = crate::services::form_ticket_service::issue(
        session,
        "home_content",
        home_content_service::fingerprint(&stored),
    )
    .await;
    render(view)
}

/// Handles the home content save.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<HomeContentForm>,
) -> Response {
    let ticket = form.form_ticket.clone();
    let input: HomeContentInput = form.into();
    // Field errors first, all at once: a refused submit
    // shows every problem together, never one per round trip.
    let validated = match home_content_service::validate_home(&input) {
        Ok(validated) => validated,
        Err(errors) => {
            return form_view(&pool, &admin, &session, input, &errors).await;
        }
    };
    // Then the pin checks that need the database (existence,
    // the event section's event-only rule).
    match home_content_service::check_pins(&pool, &validated.pins).await {
        Ok(pin_errors) if !pin_errors.is_empty() => {
            return form_view(&pool, &admin, &session, input, &pin_errors).await;
        }
        Ok(_) => {}
        Err(e) => return db_error("home content pins", e),
    }
    let Some(expected) =
        crate::services::form_ticket_service::consume(&session, "home_content", &ticket).await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to("/admin/content/home").into_response();
    };
    match home_content_service::update_home(&pool, admin.id, &validated, expected).await {
        Ok(()) => {
            set_flash(&session, "トップページ設定を保存しました").await;
            Redirect::to("/admin/content/home").into_response()
        }
        Err(crate::error::AppError::Conflict(_)) => {
            set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
            Redirect::to("/admin/content/home").into_response()
        }
        Err(e) => db_error("home content", e),
    }
}
