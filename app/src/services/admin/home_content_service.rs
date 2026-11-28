//! Admin home content rules (CMS spec C-3).
//!
//! The operator edits the top page's words: the
//! announcement bar, the hero, and each section's heading,
//! visibility and order. One form saves all of it; the save
//! is a single audited write, and a submit that changes
//! nothing writes nothing (the settings rule). Bodies and
//! texts never reach a log line — only the action name does.

use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    DATETIME_FORMAT_MESSAGE, LINK_URL_MESSAGE, SORT_ORDER_NUMERIC_MESSAGE, max_length_message,
};

use crate::models::{
    ANNOUNCEMENT_KEY, HERO_KEY, HeroImage, HomeSection, HomeSlot, PIN_SECTION_KEYS, ProductOption,
    SECTION_KEYS,
};
use crate::repositories::{home_content_repository, product_repository};
use crate::services::home_content_service::HomeContent;
use crate::services::{admin::audit_service, home_content_service, product_service};
use crate::validation::{FieldError, link_url_allowed};

/// Announcement text limit, in characters.
const ANNOUNCEMENT_TEXT_MAX_CHARS: usize = 100;
/// Hero headline limit, in characters.
const HERO_HEADLINE_MAX_CHARS: usize = 60;
/// Hero subtext limit, in characters.
const HERO_TEXT_MAX_CHARS: usize = 200;
/// Link label limit, in characters: both slots' links get
/// the cap for the hero's CTA label.
const LINK_LABEL_MAX_CHARS: usize = 20;
/// Section heading override limit, in characters.
const SECTION_HEADING_MAX_CHARS: usize = 40;
/// The hero's image cap: ten slides.
const HERO_IMAGES_MAX: usize = 10;
const SECTION_DESCRIPTION_MAX_CHARS: usize = 120;

/// The raw home content form as submitted: every value a
/// string, exactly as typed, except the switches. The window
/// fields carry the `datetime-local` text (`YYYY-MM-DDTHH:MM`)
/// the operator picked, interpreted as JST wall-clock time.
#[derive(Clone, Default)]
pub struct HomeContentInput {
    /// Announcement switch.
    pub announcement_enabled: bool,
    /// Announcement text.
    pub announcement_text: String,
    /// Announcement link label.
    pub announcement_link_label: String,
    /// Announcement link URL.
    pub announcement_link_url: String,
    /// Announcement window start, as typed.
    pub announcement_starts_at: String,
    /// Announcement window end, as typed.
    pub announcement_ends_at: String,
    /// Hero switch.
    pub hero_enabled: bool,
    /// Hero headline.
    pub hero_headline: String,
    /// Hero subtext.
    pub hero_text: String,
    /// The hero's slides, in display order.
    pub hero_images: Vec<HeroImage>,
    /// Hero window start, as typed.
    pub hero_starts_at: String,
    /// Hero window end, as typed.
    pub hero_ends_at: String,
    /// The section rows, one per canonical key.
    pub sections: Vec<HomeSectionInput>,
    /// The pin lists, one per pinnable section. A section
    /// whose editor block was not part of the submit is
    /// absent here, and its stored pins are kept.
    pub pins: Vec<PinInput>,
}

/// One section's pin list as submitted.
#[derive(Clone, Default)]
pub struct PinInput {
    /// Section key, one of the pinnable set.
    pub key: String,
    /// The pinned product ids, in display order.
    pub product_ids: Vec<i64>,
}

/// One section's form row as submitted.
#[derive(Clone, Default)]
pub struct HomeSectionInput {
    /// Section key (canonical; the form cannot invent one).
    pub key: String,
    /// Heading override as typed.
    pub heading: String,
    /// Description override as typed.
    pub description: String,
    /// Visibility switch.
    pub visible: bool,
    /// Sort position as typed.
    pub sort_order: String,
}

impl HomeContentInput {
    /// The hero images as the form's one hidden field
    /// carries them: comma-joined (the form parser rejects
    /// repeated keys, so the list rides in a single field,
    /// the pins' contract).
    pub fn hero_images_joined(&self) -> String {
        self.hero_images
            .iter()
            .map(|image| image.image_path.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The hero's slide links, joined in slide order to
    /// match [`Self::hero_images_joined`]; an unlinked
    /// slide contributes an empty entry.
    pub fn hero_image_links_joined(&self) -> String {
        self.hero_images
            .iter()
            .map(|image| image.link_url.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The hero's first image, the preview band's picture;
    /// empty when the hero has none.
    pub fn hero_first_image(&self) -> &str {
        self.hero_images
            .first()
            .map(|image| image.image_path.as_str())
            .unwrap_or("")
    }
}

impl From<&HomeContent> for HomeContentInput {
    /// The editor's initial values: the stored content, with
    /// the windows spelled as JST `datetime-local` values.
    fn from(content: &HomeContent) -> Self {
        let announcement = &content.announcement;
        let hero = &content.hero;
        Self {
            announcement_enabled: announcement.is_enabled,
            announcement_text: announcement.body.clone(),
            announcement_link_label: announcement.cta_label.clone(),
            announcement_link_url: announcement.cta_url.clone(),
            announcement_starts_at: jst_input_value(announcement.starts_at),
            announcement_ends_at: jst_input_value(announcement.ends_at),
            hero_enabled: hero.is_enabled,
            hero_headline: hero.headline.clone(),
            hero_text: hero.body.clone(),
            hero_images: content.hero_images.clone(),
            hero_starts_at: jst_input_value(hero.starts_at),
            hero_ends_at: jst_input_value(hero.ends_at),
            sections: content
                .sections
                .iter()
                .map(|section| HomeSectionInput {
                    key: section.section_key.clone(),
                    heading: section.heading.clone(),
                    description: section.description.clone(),
                    visible: section.is_visible,
                    sort_order: section.sort_order.to_string(),
                })
                .collect(),
            pins: PIN_SECTION_KEYS
                .iter()
                .map(|key| PinInput {
                    key: key.to_string(),
                    product_ids: content.pin_ids(key),
                })
                .collect(),
        }
    }
}

/// A home content form that passed validation, ready to
/// store: the two slots as full rows and every section.
pub struct ValidatedHomeContent {
    /// The announcement slot.
    pub announcement: HomeSlot,
    /// The hero slot.
    pub hero: HomeSlot,
    /// The section rows, in canonical key order.
    pub sections: Vec<HomeSection>,
    /// The pin lists to store, one per submitted pinnable
    /// section.
    pub pins: Vec<PinInput>,
    /// The hero's slides to store, in display order.
    pub hero_images: Vec<HeroImage>,
}

/// Formats a stored instant as the JST wall-clock value a
/// `datetime-local` input carries (`YYYY-MM-DDTHH:MM`);
/// an open window end formats as the empty string.
pub fn jst_input_value(at: Option<DateTime<Utc>>) -> String {
    at.map(|at| {
        at.with_timezone(&crate::models::jst::jst())
            .format("%Y-%m-%dT%H:%M")
            .to_string()
    })
    .unwrap_or_default()
}

/// Parses a `datetime-local` value as JST wall-clock time
/// and returns the instant. Seconds are accepted as well as
/// the minute form browsers submit.
fn parse_jst_local(raw: &str) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;
    naive
        .and_local_timezone(crate::models::jst::jst())
        .single()
        .map(|at| at.with_timezone(&Utc))
}

/// Reads one window field: an empty value is an open end, a
/// malformed one records the field's error and yields
/// `None` for the whole parse.
fn parse_window_field(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    raw: &str,
) -> Option<Option<DateTime<Utc>>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(None);
    }
    match parse_jst_local(raw) {
        Some(at) => Some(Some(at)),
        None => {
            errors.push(FieldError::new(field, DATETIME_FORMAT_MESSAGE));
            None
        }
    }
}

/// Checks a slot's window ordering: when both ends are set,
/// the end must be after the start — a campaign that closes
/// before it opens would silently never render.
fn check_window(
    errors: &mut Vec<FieldError>,
    end_field: &'static str,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
) {
    if let (Some(start), Some(end)) = (starts_at, ends_at)
        && end <= start
    {
        errors.push(FieldError::new(
            end_field,
            "終了日時は開始日時より後にしてください。",
        ));
    }
}

/// Pushes the length error for one field when its value is
/// over its cap.
fn check_length(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    label: &str,
    value: &str,
    max: usize,
) {
    if value.chars().count() > max {
        errors.push(FieldError::new(field, &max_length_message(label, max)));
    }
}

/// Checks one slot's link pair: label and URL are set
/// together or not at all, the label is within its cap, and
/// the URL passes the link rule (internal paths,
/// `https:` and page fragments only — never a script scheme
/// or a protocol-relative URL the layout would render as a
/// link).
fn check_link(
    errors: &mut Vec<FieldError>,
    label_field: &'static str,
    url_field: &'static str,
    label: &str,
    url: &str,
) {
    check_length(
        errors,
        label_field,
        "リンクの表示文",
        label,
        LINK_LABEL_MAX_CHARS,
    );
    if label.is_empty() && !url.is_empty() {
        errors.push(FieldError::new(
            label_field,
            "リンクの表示文を入力してください。",
        ));
    }
    if url.is_empty() && !label.is_empty() {
        errors.push(FieldError::new(
            url_field,
            "リンク先URLを入力してください。",
        ));
    }
    if !url.is_empty() && !link_url_allowed(url) {
        errors.push(FieldError::new(url_field, LINK_URL_MESSAGE));
    }
}

/// The error field name for one section's form input. The
/// keys are the fixed canonical set, so each name is a
/// literal; an unknown key falls back to the form as a
/// whole.
fn section_field(kind: &str, key: &str) -> &'static str {
    match (kind, key) {
        ("heading", "categories") => "section_heading_categories",
        ("heading", "new_arrivals") => "section_heading_new_arrivals",
        ("heading", "ranking") => "section_heading_ranking",
        ("heading", "featured") => "section_heading_featured",
        ("heading", "staff_picks") => "section_heading_staff_picks",
        ("heading", "notices") => "section_heading_notices",
        ("heading", "free_space") => "section_heading_free_space",
        ("sort", "free_space") => "section_sort_free_space",
        ("description", "free_space") => "section_description_free_space",
        ("description", "categories") => "section_description_categories",
        ("description", "new_arrivals") => "section_description_new_arrivals",
        ("description", "ranking") => "section_description_ranking",
        ("description", "featured") => "section_description_featured",
        ("description", "staff_picks") => "section_description_staff_picks",
        ("description", "notices") => "section_description_notices",
        ("sort", "categories") => "section_sort_categories",
        ("sort", "new_arrivals") => "section_sort_new_arrivals",
        ("sort", "ranking") => "section_sort_ranking",
        ("sort", "featured") => "section_sort_featured",
        ("sort", "staff_picks") => "section_sort_staff_picks",
        ("sort", "notices") => "section_sort_notices",
        _ => "sections",
    }
}

/// The error field name for one section's pin list.
fn pin_field(key: &str) -> &'static str {
    match key {
        "new_arrivals" => "pins_new_arrivals",
        "ranking" => "pins_ranking",
        "event" => "pins_event",
        "featured" => "pins_featured",
        _ => "pins",
    }
}

/// Validates the home content form, reporting every failing
/// field at once. Values are trimmed before they are
/// checked and stored (the window text is parsed, not
/// trimmed into the database).
pub fn validate_home(input: &HomeContentInput) -> Result<ValidatedHomeContent, Vec<FieldError>> {
    let mut errors = Vec::new();

    let announcement_starts = parse_window_field(
        &mut errors,
        "announcement_starts_at",
        &input.announcement_starts_at,
    );
    let announcement_ends = parse_window_field(
        &mut errors,
        "announcement_ends_at",
        &input.announcement_ends_at,
    );
    let hero_starts = parse_window_field(&mut errors, "hero_starts_at", &input.hero_starts_at);
    let hero_ends = parse_window_field(&mut errors, "hero_ends_at", &input.hero_ends_at);

    let announcement_text = input.announcement_text.trim().to_string();
    if input.announcement_enabled && announcement_text.is_empty() {
        errors.push(FieldError::new(
            "announcement_text",
            "お知らせ文を入力してください。",
        ));
    }
    check_length(
        &mut errors,
        "announcement_text",
        "お知らせ文",
        &announcement_text,
        ANNOUNCEMENT_TEXT_MAX_CHARS,
    );
    check_link(
        &mut errors,
        "announcement_link_label",
        "announcement_link_url",
        input.announcement_link_label.trim(),
        input.announcement_link_url.trim(),
    );

    // The headline is optional: an
    // enabled hero without one simply omits it, and a hero
    // with no content at all does not render.
    let hero_headline = input.hero_headline.trim().to_string();
    check_length(
        &mut errors,
        "hero_headline",
        "見出し",
        &hero_headline,
        HERO_HEADLINE_MAX_CHARS,
    );
    check_length(
        &mut errors,
        "hero_text",
        "本文",
        input.hero_text.trim(),
        HERO_TEXT_MAX_CHARS,
    );
    // The hero images attach from the uploads store under
    // the product form's rule: a path naming no real
    // uploaded file is refused, so the storefront can never
    // render a hero image that is not there. At most ten,
    // and never the same image twice.
    let mut hero_images: Vec<HeroImage> = Vec::new();
    for raw in &input.hero_images {
        let path = raw.image_path.trim().to_string();
        let link_url = raw.link_url.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !crate::services::admin::product_service::is_upload_path(&path) {
            errors.push(FieldError::new(
                "hero_images",
                "画像ファイルが見つかりません。アップロードし直してください。",
            ));
            break;
        }
        if hero_images.iter().any(|image| image.image_path == path) {
            errors.push(FieldError::new("hero_images", "同じ画像が重複しています。"));
            break;
        }
        if !link_url.is_empty() && !link_url_allowed(&link_url) {
            errors.push(FieldError::new("hero_images", LINK_URL_MESSAGE));
            break;
        }
        hero_images.push(HeroImage {
            image_path: path,
            link_url,
        });
    }
    if hero_images.len() > HERO_IMAGES_MAX {
        errors.push(FieldError::new(
            "hero_images",
            "ヒーロー画像は10枚までです。",
        ));
    }

    if let (Some(starts), Some(ends)) = (announcement_starts, announcement_ends) {
        check_window(&mut errors, "announcement_ends_at", starts, ends);
    }
    if let (Some(starts), Some(ends)) = (hero_starts, hero_ends) {
        check_window(&mut errors, "hero_ends_at", starts, ends);
    }

    let mut sections = Vec::new();
    for row in &input.sections {
        if !SECTION_KEYS.contains(&row.key.as_str()) {
            continue;
        }
        let heading = row.heading.trim().to_string();
        check_length(
            &mut errors,
            section_field("heading", &row.key),
            "見出し",
            &heading,
            SECTION_HEADING_MAX_CHARS,
        );
        let description = row.description.trim().to_string();
        check_length(
            &mut errors,
            section_field("description", &row.key),
            "説明文",
            &description,
            SECTION_DESCRIPTION_MAX_CHARS,
        );
        let sort_order = match row.sort_order.trim().parse::<i32>() {
            Ok(value) => value,
            Err(_) => {
                errors.push(FieldError::new(
                    section_field("sort", &row.key),
                    SORT_ORDER_NUMERIC_MESSAGE,
                ));
                0
            }
        };
        sections.push(HomeSection {
            section_key: row.key.clone(),
            heading,
            description,
            is_visible: row.visible,
            sort_order,
            updated_at: input_updated_at(),
        });
    }

    let mut pins = Vec::new();
    for pin in &input.pins {
        if !PIN_SECTION_KEYS.contains(&pin.key.as_str()) {
            continue;
        }
        let field = pin_field(&pin.key);
        let mut seen = std::collections::HashSet::new();
        if pin.product_ids.iter().any(|id| !seen.insert(*id)) {
            errors.push(FieldError::new(
                field,
                "固定表示に同じ商品が重複しています。",
            ));
        }
        let cap = product_service::pin_cap(&pin.key).unwrap_or(0);
        if cap > 0 && pin.product_ids.len() as i64 > cap {
            errors.push(FieldError::new(
                field,
                &format!("固定表示は最大{cap}件までです。"),
            ));
        }
        pins.push(pin.clone());
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    // Every window parsed: an unparseable one pushed an
    // error above, so the outer Options are all Some here
    // and flattening only opens the window ends.
    Ok(ValidatedHomeContent {
        announcement: HomeSlot {
            key: ANNOUNCEMENT_KEY.to_string(),
            headline: String::new(),
            body: announcement_text,
            cta_label: input.announcement_link_label.trim().to_string(),
            cta_url: input.announcement_link_url.trim().to_string(),
            is_enabled: input.announcement_enabled,
            starts_at: announcement_starts.flatten(),
            ends_at: announcement_ends.flatten(),
            updated_at: input_updated_at(),
        },
        hero: HomeSlot {
            key: HERO_KEY.to_string(),
            headline: hero_headline,
            body: input.hero_text.trim().to_string(),
            // The hero's link fields are retired: its
            // slides carry the links now, so the slot's
            // pair is stored empty.
            cta_label: String::new(),
            cta_url: String::new(),
            is_enabled: input.hero_enabled,
            starts_at: hero_starts.flatten(),
            ends_at: hero_ends.flatten(),
            updated_at: input_updated_at(),
        },
        sections,
        pins,
        hero_images,
    })
}

/// The `updated_at` a validated row carries until the
/// repository stamps the real one: the value is never read
/// before it is overwritten, so the current instant stands
/// in.
fn input_updated_at() -> DateTime<Utc> {
    Utc::now()
}

/// Returns the stored home content for the editor, read
/// fresh: an admin must see the stored truth, never the
/// storefront's brief cache.
pub async fn get_home(pool: &PgPool) -> Result<HomeContent, AppError> {
    let slots = home_content_repository::list_slots(pool).await?;
    let sections = home_content_repository::list_sections(pool).await?;
    let pins = home_content_repository::list_pins(pool).await?;
    let hero_images = home_content_repository::list_hero_images(pool).await?;
    Ok(home_content_service::assemble(
        slots,
        sections,
        pins,
        hero_images,
    ))
}

/// Whether two slot rows carry the same content (the
/// `updated_at` stamp is not content).
fn slot_content_eq(a: &HomeSlot, b: &HomeSlot) -> bool {
    a.key == b.key
        && a.headline == b.headline
        && a.body == b.body
        && a.cta_label == b.cta_label
        && a.cta_url == b.cta_url
        && a.is_enabled == b.is_enabled
        && a.starts_at == b.starts_at
        && a.ends_at == b.ends_at
}

/// Whether two section rows carry the same content.
fn section_content_eq(a: &HomeSection, b: &HomeSection) -> bool {
    a.section_key == b.section_key
        && a.heading == b.heading
        && a.description == b.description
        && a.is_visible == b.is_visible
        && a.sort_order == b.sort_order
}

/// Stores validated home content and audits the change in
/// the same transaction, then drops the storefront cache so
/// the new words are live on the next page. A submit that
/// changes nothing writes nothing.
/// The form-ticket stamp for the home content editor: a
/// fingerprint of everything the save compares — both
/// slots, every section, the pins and the hero slides
/// (the save replaces or rewrites all of it, so the whole
/// set is the record). Unlike the single-record guards,
/// the comparison runs on the read at the top of
/// `update_home`, before its transaction — it narrows
/// the race to the save's own duration rather than
/// eliminating it, proportionate for a whole-set replace.
pub fn fingerprint(content: &HomeContent) -> i64 {
    let mut stamp = crate::services::form_ticket_service::SetStamp::new();
    for slot in [&content.announcement, &content.hero] {
        stamp.text(&slot.key);
        stamp.text(&slot.headline);
        stamp.text(&slot.body);
        stamp.text(&slot.cta_label);
        stamp.text(&slot.cta_url);
        stamp.flag(slot.is_enabled);
        stamp.int(slot.starts_at.map_or(0, |at| at.timestamp_micros()));
        stamp.int(slot.ends_at.map_or(0, |at| at.timestamp_micros()));
    }
    for section in &content.sections {
        stamp.text(&section.section_key);
        stamp.text(&section.heading);
        stamp.text(&section.description);
        stamp.flag(section.is_visible);
        stamp.int(i64::from(section.sort_order));
    }
    for pin in &content.pins {
        stamp.text(&pin.section_key);
        stamp.int(pin.product_id);
        stamp.int(i64::from(pin.sort_order));
    }
    for image in &content.hero_images {
        stamp.text(&image.image_path);
        stamp.text(&image.link_url);
    }
    stamp.finish()
}

pub async fn update_home(
    pool: &PgPool,
    admin_id: i64,
    validated: &ValidatedHomeContent,
    expected: i64,
) -> Result<(), AppError> {
    let current = get_home(pool).await?;
    if fingerprint(&current) != expected {
        return Err(crate::services::form_ticket_service::stale_conflict());
    }
    let pins_changed = validated
        .pins
        .iter()
        .any(|pin| current.pin_ids(&pin.key) != pin.product_ids);
    let hero_images_changed = current.hero_images != validated.hero_images;
    let changed = !slot_content_eq(&current.announcement, &validated.announcement)
        || !slot_content_eq(&current.hero, &validated.hero)
        || current.sections.len() != validated.sections.len()
        || current
            .sections
            .iter()
            .zip(validated.sections.iter())
            .any(|(stored, next)| !section_content_eq(stored, next))
        || pins_changed
        || hero_images_changed;
    if !changed {
        return Ok(());
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    home_content_repository::update_slot(&mut tx, &validated.announcement, admin_id).await?;
    home_content_repository::update_slot(&mut tx, &validated.hero, admin_id).await?;
    home_content_repository::replace_hero_images(&mut tx, &validated.hero_images).await?;
    for section in &validated.sections {
        home_content_repository::update_section(&mut tx, section, admin_id).await?;
    }
    for pin in &validated.pins {
        home_content_repository::replace_pins(&mut tx, &pin.key, &pin.product_ids).await?;
    }
    audit_service::record(&mut tx, admin_id, "home.update", "home_content", Some(1)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // Storefront reads go through the content cache; drop it
    // so the save is live on the next page.
    home_content_service::invalidate_cache();
    tracing::info!(admin_id, "home.update: home content changed");
    Ok(())
}

/// The database-backed pin checks, run after the pure
/// validation: every pinned product must exist, and the
/// event section may only pin event products (its row is
/// the venue-pickup row — a non-event product there would
/// promise a pickup the product does not offer).
pub async fn check_pins(pool: &PgPool, pins: &[PinInput]) -> Result<Vec<FieldError>, AppError> {
    let mut errors = Vec::new();
    for pin in pins {
        if pin.product_ids.is_empty() {
            continue;
        }
        let field = pin_field(&pin.key);
        let facts = product_repository::pin_facts(pool, &pin.product_ids).await?;
        if facts.len() != pin.product_ids.len() {
            errors.push(FieldError::new(
                field,
                "固定表示に存在しない商品が含まれています。",
            ));
            continue;
        }
        if pin.key == "event" && facts.iter().any(|(_, is_event)| !is_event) {
            errors.push(FieldError::new(
                field,
                "イベント受取に固定できるのは、イベント商品だけです。",
            ));
        }
    }
    Ok(errors)
}

/// One pin editor's resolved rows, for the settings page.
pub struct PinEditorData {
    /// Section key, one of the pinnable set.
    pub key: String,
    /// The current pins with their product names and
    /// statuses, in pin order.
    pub rows: Vec<PinEditorRow>,
}

/// One resolved pin row in the editor.
pub struct PinEditorRow {
    /// The pinned product.
    pub product_id: i64,
    /// The pinned product's name.
    pub name: String,
    /// The pinned product's lifecycle status.
    pub status: String,
}

/// Resolves the editor's pin rows and the product options
/// the add-picker offers: one read of every product, used
/// for both — the row names and the picker share the truth.
pub async fn pin_editor_data(
    pool: &PgPool,
    input: &HomeContentInput,
) -> Result<(Vec<PinEditorData>, Vec<ProductOption>), AppError> {
    let options = home_content_repository::list_product_options(pool).await?;
    let editors = input
        .pins
        .iter()
        .map(|pin| PinEditorData {
            key: pin.key.clone(),
            rows: pin
                .product_ids
                .iter()
                .filter_map(|id| {
                    options
                        .iter()
                        .find(|option| option.id == *id)
                        .map(|option| PinEditorRow {
                            product_id: option.id,
                            name: option.name.clone(),
                            status: option.status.clone(),
                        })
                })
                .collect(),
        })
        .collect();
    Ok((editors, options))
}

#[cfg(test)]
mod tests {
    use super::fingerprint;
    use crate::models::{HeroImage, HomePin, HomeSection, HomeSlot};
    use crate::services::home_content_service::HomeContent;

    /// A slot with the given headline.
    fn slot(key: &str, headline: &str) -> HomeSlot {
        HomeSlot {
            key: key.to_string(),
            headline: headline.to_string(),
            body: String::new(),
            cta_label: String::new(),
            cta_url: String::new(),
            is_enabled: true,
            starts_at: None,
            ends_at: None,
            updated_at: chrono::DateTime::UNIX_EPOCH,
        }
    }

    /// A minimal assembled home content.
    fn content() -> HomeContent {
        HomeContent {
            announcement: slot("announcement", ""),
            hero: slot("hero", "タイトル"),
            sections: vec![HomeSection {
                section_key: "featured".to_string(),
                heading: "おすすめ".to_string(),
                description: String::new(),
                is_visible: true,
                sort_order: 0,
                updated_at: chrono::DateTime::UNIX_EPOCH,
            }],
            pins: vec![HomePin {
                section_key: "featured".to_string(),
                product_id: 42,
                sort_order: 0,
            }],
            hero_images: vec![HeroImage {
                image_path: "hero/1.jpg".to_string(),
                link_url: String::new(),
            }],
        }
    }

    #[test]
    fn home_fingerprint_is_stable_and_reacts_to_edits() {
        assert_eq!(fingerprint(&content()), fingerprint(&content()));

        let mut retitled = content();
        retitled.hero.headline = "新しいタイトル".to_string();
        assert_ne!(fingerprint(&content()), fingerprint(&retitled));

        let mut repinned = content();
        repinned.pins[0].product_id = 43;
        assert_ne!(fingerprint(&content()), fingerprint(&repinned));

        let mut reimaged = content();
        reimaged.hero_images[0].image_path = "hero/2.jpg".to_string();
        assert_ne!(fingerprint(&content()), fingerprint(&reimaged));
    }
}
