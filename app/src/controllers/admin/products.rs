//! Admin product management.
//!
//! List with tabs/filters, the shared create/edit form, the
//! archive actions, the bulk status bar, and the image upload
//! endpoint the form's picker feeds from. Products are
//! never hard-deleted here — order history refers to them — so
//! the only removal path is archiving. Every write goes through
//! [`crate::services::admin::product_service`],
//! which audits inside the same transaction.

use axum::{
    Form, Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{
    db_error, key_parts, layout_for, pagination, record_not_found, render, safe_return_url,
    set_flash,
};
use crate::config::CONFIG;
use crate::messages::{
    CSV_FILE_REQUIRED_MESSAGE, CSV_FILE_TOO_LARGE_MESSAGE, CSV_IMPORT_DISCARDED_NOTICE,
    IMAGE_FILE_REQUIRED_MESSAGE, IMAGE_LOAD_FAILED_MESSAGE, csv_import_applied_message,
};
use crate::models::AdminIdentity;
use crate::repositories::admin::product_repository;
use crate::repositories::csv_import_batch_repository;
use crate::services::admin::product_csv_service;
use crate::services::admin::product_service::{
    self, AxisFormInput, OptionFormInput, ProductFormInput,
};
use crate::session::CurrentAdmin;
use crate::util::csv_transfer::BatchReport;
use crate::views::admin::products::{
    CategoryOption, FormAxis, FormImage, FormOption, ProductCsvImportView, ProductCsvPreviewView,
    ProductFormView, ProductListView, SortOption, TagCheck, TagOption,
};
use crate::views::admin::{Page, filter_query};

/// Most product IDs one bulk POST may carry. The selection bar
/// offers one page of rows; the cap exists for crafted POSTs,
/// not for the UI.
const MAX_BULK_IDS: usize = 200;

/// Body ceiling for the image upload route only: the 20 MB
/// image cap (see `product_service::MAX_IMAGE_BYTES`)
/// plus room for the multipart framing around the file.
const UPLOAD_BODY_LIMIT: usize = 24 * 1024 * 1024;

/// Routes for the whole product surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/products", get(list).post(bulk))
        .route("/admin/products/new", get(new_form).post(create))
        .route(
            "/admin/products/image-upload",
            post(upload_image).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        .route("/admin/products/{id}/edit", get(edit_form).post(update))
        .route("/admin/products/{id}/stock-history", get(stock_history))
        .route("/admin/products/csv-export", get(csv_export))
        .route(
            "/admin/products/csv-import",
            get(csv_import_form)
                .post(csv_upload)
                .layer(DefaultBodyLimit::max(CSV_BODY_LIMIT)),
        )
        .route("/admin/products/csv-import/{id}", get(csv_preview))
        .route("/admin/products/csv-import/{id}/apply", post(csv_apply))
        .route("/admin/products/csv-import/{id}/discard", post(csv_discard))
        .route("/admin/products/{id}/archive", post(archive))
        .route("/admin/products/{id}/unarchive", post(unarchive))
}

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Keyword (partial match on name + category).
    q: Option<String>,
    /// Category exact filter.
    category: Option<String>,
    /// Tag ID filter.
    tag: Option<String>,
    /// Sort key.
    sort: Option<String>,
    /// Tab key.
    tab: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// The sort allowlist: (value, label), in order.
const SORTS: [(&str, &str); 8] = [
    ("new", "新しい順"),
    ("old", "古い順"),
    ("updated", "更新の新しい順"),
    ("name", "商品名順"),
    ("price_asc", "価格の安い順"),
    ("price_desc", "価格の高い順"),
    ("stock_asc", "在庫の少ない順"),
    ("stock_desc", "在庫の多い順"),
];

/// The product list page.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    let tab = match query.tab.as_deref() {
        Some("public") => "public",
        Some("draft") => "draft",
        Some("archived") => "archived",
        _ => "all",
    };
    let sort = query
        .sort
        .as_deref()
        .filter(|s| SORTS.iter().any(|(v, _)| v == s))
        .unwrap_or("new");
    let q = query.q.clone().unwrap_or_default();
    let keyword = crate::util::like_pattern::like_pattern(&q);
    let category = query.category.clone().filter(|c| !c.trim().is_empty());
    let tag_id = query.tag.as_deref().and_then(|t| t.parse::<i64>().ok());
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);

    let (rows, total) = match product_service::list_products_page(
        &pool,
        &product_service::AdminProductQuery {
            tab,
            keyword: keyword.clone(),
            category: category.clone(),
            tag_id,
            sort,
            limit,
            offset,
        },
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    // The tab counts, filter categories and tags are page data
    // from the database: a failed query is a 500, never zeroed
    // counts and empty filters rendered as if real (the same
    // error-swallowing class as the dashboard and customer
    // totals fixes of 2026-11-02).
    let (count_all, count_public, count_draft, count_archived) =
        match product_service::tab_counts(&pool).await {
            Ok(v) => v,
            Err(e) => return db_error("products", e),
        };
    let categories = match product_service::list_categories(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    let tags = match product_service::list_tags(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };

    // The tab links carry the filters WITHOUT the tab (they
    // set it themselves); the pagination base carries them
    // WITH it, so paging stays inside the active tab.
    let q_param = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let category_param = query
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tag_param = query.tag.as_deref().filter(|s| !s.is_empty());
    let filters = filter_query(&[
        ("q", q_param),
        ("category", category_param),
        ("tag", tag_param),
        ("sort", Some(sort)),
    ]);
    let page_base = Page::base_with(
        "/admin/products",
        &filter_query(&[
            ("tab", Some(tab)),
            ("q", q_param),
            ("category", category_param),
            ("tag", tag_param),
            ("sort", Some(sort)),
        ]),
    );
    let page_text = page_no.to_string();
    let bulk_return_url = format!(
        "/admin/products?{}",
        filter_query(&[
            ("tab", Some(tab)),
            ("q", q_param),
            ("category", category_param),
            ("tag", tag_param),
            ("sort", Some(sort)),
            ("page", (page_no > 1).then_some(page_text.as_str())),
        ])
    );
    let csv_export_url = format!(
        "/admin/products/csv-export?{}",
        filter_query(&[
            ("tab", Some(tab)),
            ("q", q_param),
            ("category", category_param),
            ("tag", tag_param),
            ("sort", Some(sort)),
        ])
    );
    render(ProductListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "products",
        tab: tab.to_string(),
        count_all,
        count_public,
        count_draft,
        count_archived,
        tab_all_url: format!("/admin/products?tab=all&{filters}"),
        tab_public_url: format!("/admin/products?tab=public&{filters}"),
        tab_draft_url: format!("/admin/products?tab=draft&{filters}"),
        tab_archived_url: format!("/admin/products?tab=archived&{filters}"),
        csv_export_url,
        bulk_return_url,
        q,
        products: rows
            .into_iter()
            .map(|p| crate::views::admin::products::ProductRow {
                price_text: crate::pricing::display_price(p.price_yen),
                updated_text: crate::models::jst::jst_datetime_label(p.updated_at),
                product: p,
            })
            .collect(),
        categories: categories
            .iter()
            .map(|c| CategoryOption {
                value: c.clone(),
                selected: category.as_deref() == Some(c.as_str()),
            })
            .collect(),
        tags: tags
            .iter()
            .map(|t| TagOption {
                id: t.id,
                label: t.label.clone(),
                selected: tag_id == Some(t.id),
            })
            .collect(),
        sorts: SORTS
            .iter()
            .map(|(v, l)| SortOption {
                value: v,
                label: l,
                selected: sort == *v,
            })
            .collect(),
        page: Page::new(page_no, total, limit, page_base),
    })
}

/// A blank create form.
pub async fn new_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let input = ProductFormInput {
        status: "draft".to_string(),
        tax_class_id: "1".to_string(),
        track_stock: true,
        ..Default::default()
    };
    form_response(&pool, &admin, &session, &input, None, None).await
}

/// The product and bulk forms are read as raw (name, value)
/// pairs rather than a derived struct: the tag checkboxes and
/// the bulk selection repeat one field name, and
/// serde_urlencoded cannot build a `Vec` from repeated keys
/// (it answers 422, "expected a sequence"). Pairs keep every
/// value, in order, with no new dependencies.
type FormPairs = Vec<(String, String)>;

/// The first value of one form field, or empty when absent.
fn form_field(pairs: &FormPairs, name: &str) -> String {
    pairs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// Every value of one repeated form field (checkbox groups).
fn form_values(pairs: &FormPairs, name: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .collect()
}

/// Builds the variation axes from the raw pairs. The editor
/// posts indexed fields (`axes[i][name]`,
/// `axes[i][options][j][value]`, ...); indices order the rows,
/// and gaps from a removed row simply collapse.
fn axes_from(pairs: &FormPairs) -> Vec<AxisFormInput> {
    use std::collections::BTreeMap;
    let mut axes: BTreeMap<usize, AxisFormInput> = BTreeMap::new();
    let mut options: BTreeMap<usize, BTreeMap<usize, OptionFormInput>> = BTreeMap::new();
    for (key, value) in pairs {
        let parts = key_parts(key);
        if parts.first().map(String::as_str) != Some("axes") {
            continue;
        }
        let Some(axis_idx) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) else {
            continue;
        };
        match parts.len() {
            3 if parts[2] == "id" => axes.entry(axis_idx).or_default().id = value.clone(),
            3 if parts[2] == "name" => axes.entry(axis_idx).or_default().name = value.clone(),
            5 if parts[2] == "options" => {
                let Some(option_idx) = parts[3].parse::<usize>().ok() else {
                    continue;
                };
                let slot = options
                    .entry(axis_idx)
                    .or_default()
                    .entry(option_idx)
                    .or_default();
                match parts[4].as_str() {
                    "id" => slot.id = value.clone(),
                    "value" => slot.value = value.clone(),
                    _ => {}
                }
                axes.entry(axis_idx).or_default();
            }
            _ => {}
        }
    }
    axes.into_iter()
        .map(|(idx, mut axis)| {
            axis.options = options
                .remove(&idx)
                .map(|m| m.into_values().collect())
                .unwrap_or_default();
            axis
        })
        .collect()
}

/// The combination field sets from the raw pairs, in the
/// grid's posted order (`combo_stock[0]`, `combo_price[0]`,
/// ...). An index appears when any of its fields was posted;
/// the enable checkbox posts only when checked, so its
/// absence at a posted index reads as disabled.
fn combos_from(pairs: &FormPairs) -> Vec<product_service::ComboFormInput> {
    use std::collections::BTreeMap;
    let mut combos: BTreeMap<usize, product_service::ComboFormInput> = BTreeMap::new();
    for (key, value) in pairs {
        let parts = key_parts(key);
        if parts.len() != 2 {
            continue;
        }
        let Ok(idx) = parts[1].parse::<usize>() else {
            continue;
        };
        let slot = combos.entry(idx).or_default();
        match parts[0].as_str() {
            "combo_stock" => slot.stock = value.clone(),
            "combo_price" => slot.price_yen = value.clone(),
            "combo_sku" => slot.sku = value.clone(),
            "combo_jan" => slot.jan_code = value.clone(),
            "combo_image" => slot.image_path = value.clone(),
            "combo_enabled" => slot.is_enabled = value == "1",
            "combo_incoming" => slot.incoming_qty = value.clone(),
            "combo_incoming_date" => slot.incoming_date = value.clone(),
            _ => {}
        }
    }
    combos.into_values().collect()
}

/// An existing value's JST `datetime-local` rendering (the
/// form's datetime fields are JST wall-clock).
fn jst_local(value: Option<chrono::DateTime<chrono::Utc>>) -> String {
    value
        .map(|d| {
            (d + chrono::Duration::hours(9))
                .format("%Y-%m-%dT%H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// An optional integer's form rendering.
fn opt_int<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

/// Loads the validation's tax classes (with today's rates) and
/// taken-identifier sets; either failure propagates like the
/// other admin vocabulary loads.
async fn tax_and_taken(
    pool: &PgPool,
    exclude_product_id: Option<i64>,
) -> Result<
    (
        Vec<crate::models::tax::ClassRate>,
        product_repository::TakenIdentifiers,
    ),
    crate::error::AppError,
> {
    let classes =
        crate::repositories::tax_repository::class_rates_on(pool, crate::models::jst::jst_today())
            .await
            .map_err(|e| crate::error::AppError::db("load tax classes", e))?;
    let taken = product_repository::taken_identifiers(pool, exclude_product_id).await?;
    Ok((classes, taken))
}

/// Builds the service's form input from the raw pairs.
fn input_from(pairs: &FormPairs) -> ProductFormInput {
    let checked = |name: &str| pairs.iter().any(|(k, _)| k == name);
    ProductFormInput {
        name: form_field(pairs, "name"),
        description: form_field(pairs, "description"),
        category_id: form_field(pairs, "category_id"),
        price_yen: form_field(pairs, "price_yen"),
        price_mode: form_field(pairs, "price_mode"),
        price_excl_yen: form_field(pairs, "price_excl_yen"),
        cost_yen: form_field(pairs, "cost_yen"),
        tax_class_id: form_field(pairs, "tax_class_id"),
        sale_price_yen: form_field(pairs, "sale_price_yen"),
        sale_starts_at: form_field(pairs, "sale_starts_at"),
        sale_ends_at: form_field(pairs, "sale_ends_at"),
        product_code: form_field(pairs, "product_code"),
        brand: form_field(pairs, "brand"),
        manufacturer: form_field(pairs, "manufacturer"),
        country_of_origin: form_field(pairs, "country_of_origin"),
        spec_text: form_field(pairs, "spec_text"),
        features_text: form_field(pairs, "features_text"),
        description_sp: form_field(pairs, "description_sp"),
        search_keywords: form_field(pairs, "search_keywords"),
        is_new: checked("is_new"),
        is_preorder: checked("is_preorder"),
        is_staff_pick: checked("is_staff_pick"),
        discount_percent: form_field(pairs, "discount_percent"),
        display_order: form_field(pairs, "display_order"),
        slug: form_field(pairs, "slug"),
        seo_title: form_field(pairs, "seo_title"),
        meta_description: form_field(pairs, "meta_description"),
        track_stock: checked("track_stock"),
        allow_backorder: checked("allow_backorder"),
        low_stock_threshold: form_field(pairs, "low_stock_threshold"),
        points_earn_percent: form_field(pairs, "points_earn_percent"),
        weight_grams: form_field(pairs, "weight_grams"),
        size_length_mm: form_field(pairs, "size_length_mm"),
        size_width_mm: form_field(pairs, "size_width_mm"),
        size_height_mm: form_field(pairs, "size_height_mm"),
        cool_delivery: checked("cool_delivery"),
        mail_delivery: checked("mail_delivery"),
        free_shipping: checked("free_shipping"),
        undeliverable_regions: form_field(pairs, "undeliverable_regions"),
        lead_time_days: form_field(pairs, "lead_time_days"),
        publish_starts_at: form_field(pairs, "publish_starts_at"),
        publish_ends_at: form_field(pairs, "publish_ends_at"),
        axes: axes_from(pairs),
        combos: {
            let mut combos = combos_from(pairs);
            // A post carrying none of the combination upgrade
            // keys predates them: every combination is enabled,
            // as all combinations were before the toggle existed.
            // (The current form always posts combo_enabled,
            // 0 or 1, so its deliberate unchecks are honoured.)
            let legacy = !pairs.iter().any(|(k, _)| k.starts_with("combo_enabled"));
            if legacy {
                for combo in &mut combos {
                    combo.is_enabled = true;
                }
            }
            combos
        },
        // The 下書きで保存 button posts save_as=draft and wins
        // over the status select.
        status: if form_field(pairs, "save_as") == "draft" {
            "draft".to_string()
        } else {
            form_field(pairs, "status")
        },
        is_event: checked("event"),
        images: form_values(pairs, "images"),
        image_alts: form_values(pairs, "image_alt"),
        tag_ids: form_values(pairs, "tag_ids")
            .iter()
            .filter_map(|v| v.parse::<i64>().ok())
            .collect(),
    }
}

/// Handles the create submit: all errors at once on failure,
/// flash + redirect to the new product's edit screen on
/// success, so follow-up edits (variants, images) continue
/// in context.
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<FormPairs>,
) -> Response {
    let mut input = input_from(&pairs);
    // A post that predates the upgrade fields carries no tax
    // class: default to the standard class, and to stock
    // tracking on (the long-standing behaviour).
    if !pairs.iter().any(|(k, _)| k == "tax_class_id") {
        input.tax_class_id = "1".to_string();
    }
    if !pairs.iter().any(|(k, _)| k == "track_stock") {
        input.track_stock = true;
    }
    // The tag list is the validation vocabulary: swallowing a
    // failed query would reject every submitted tag as unknown,
    // so the error propagates like the other admin queries.
    let tags = match product_service::list_tags(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    let valid_ids: Vec<i64> = tags.iter().map(|t| t.id).collect();
    // The category vocabulary is the managed table: the
    // submitted ID is re-checked against it by the validator.
    let categories = match product_service::list_category_records(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    let (tax_classes, taken) = match tax_and_taken(&pool, None).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    match product_service::validate_product(
        &input,
        &[],
        &valid_ids,
        &categories,
        None,
        None,
        &tax_classes,
        &taken,
    ) {
        Ok(valid) => match product_service::create_product(&pool, admin.id, &valid, None).await {
            Ok(id) => {
                set_flash(&session, "商品を登録しました").await;
                Redirect::to(&format!("/admin/products/{id}/edit")).into_response()
            }
            Err(e) => db_error("products", e),
        },
        Err(errors) => {
            let by_field = product_service::errors_by_field(&errors);
            form_response(&pool, &admin, &session, &input, None, Some(&by_field)).await
        }
    }
}

/// Receives one product image upload (multipart, field
/// `image`), stores it under a generated name through
/// the service, and answers the JSON the form script
/// consumes: `name`, the picker value for the stored file,
/// and `url`, its served path. Failures answer a non-2xx
/// status with a JSON `message` — the script shows that
/// string to the admin as-is.
///
/// The token rides in the `x-csrf-token` header on purpose:
/// the CSRF middleware validates a header token without
/// buffering the body, so the multipart stream reaches this
/// handler whole — a form-field token would cap the upload
/// at the middleware's form-body limit. The admin session
/// is required by the [`CurrentAdmin`] extractor, as on
/// every admin write.
pub async fn upload_image(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    mut multipart: Multipart,
) -> Response {
    let mut bytes: Option<Vec<u8>> = None;
    let mut original_name: Option<String> = None;
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                if field.name() == Some("image") {
                    // The client's original filename,
                    // kept only so the stored name can
                    // carry a recognisable prefix from it
                    // (the service rebuilds it safely).
                    original_name = field.file_name().map(str::to_owned);
                    match field.bytes().await {
                        Ok(data) => bytes = Some(data.to_vec()),
                        Err(e) => {
                            tracing::warn!("product image upload body read failed: {e}");
                            return upload_error(
                                StatusCode::BAD_REQUEST,
                                IMAGE_LOAD_FAILED_MESSAGE,
                            );
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("product image upload parse failed: {e}");
                return upload_error(StatusCode::BAD_REQUEST, IMAGE_LOAD_FAILED_MESSAGE);
            }
        }
    }
    let Some(bytes) = bytes else {
        return upload_error(StatusCode::BAD_REQUEST, IMAGE_FILE_REQUIRED_MESSAGE);
    };
    match product_service::store_uploaded_image(&pool, admin.id, original_name.as_deref(), &bytes)
        .await
    {
        Ok(choice) => (
            StatusCode::OK,
            Json(serde_json::json!({ "name": choice.filename, "url": choice.path })),
        )
            .into_response(),
        Err(product_service::UploadError::Refused(message)) => {
            upload_error(StatusCode::BAD_REQUEST, &message)
        }
        Err(product_service::UploadError::Internal) => upload_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "アップロードに失敗しました。時間をおいてもう一度お試しください",
        ),
    }
}

/// One upload failure as a JSON `message`: the form script
/// shows the string to the admin as-is, so it is already the
/// user-facing wording, never markup.
fn upload_error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "message": message }))).into_response()
}

/// The edit form for one product. Archived products render the
/// form read-only with the unarchive action; a missing ID is
/// the styled admin 404.
pub async fn edit_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let product = match product_service::find_product(&pool, id).await {
        Ok(Some(product)) => product,
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("products", e),
    };
    // A swallowed failure here would render the form with no
    // tags checked — and saving it would strip the product's
    // tags — so the error propagates instead.
    let tag_ids = match product_service::product_tag_ids(&pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    // The stored matrix fills the variation editor; a failed
    // load must not render an empty editor whose save would wipe
    // the product's combinations, so the error propagates.
    let matrix = match product_service::existing_matrix(&pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    // The attached image list fills the gallery; a failed load
    // must not render an empty gallery whose save would strip
    // the product's images, so the error propagates.
    let attached_detailed = match product_service::product_images_detailed(&pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    let attached: Vec<String> = attached_detailed
        .iter()
        .map(|(path, _)| path.clone())
        .collect();
    let input = ProductFormInput {
        name: product.name.clone(),
        description: product.description.clone(),
        category_id: product
            .category_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        price_yen: product.price_yen.to_string(),
        price_mode: "incl".to_string(),
        price_excl_yen: String::new(),
        cost_yen: opt_int(product.cost_yen),
        tax_class_id: product.tax_class_id.to_string(),
        sale_price_yen: opt_int(product.sale_price_yen),
        sale_starts_at: jst_local(product.sale_starts_at),
        sale_ends_at: jst_local(product.sale_ends_at),
        product_code: product.product_code.clone().unwrap_or_default(),
        brand: product.brand.clone(),
        manufacturer: product.manufacturer.clone(),
        country_of_origin: product.country_of_origin.clone(),
        spec_text: product.spec_text.clone(),
        features_text: product.features_text.clone(),
        description_sp: product.description_sp.clone(),
        search_keywords: product.search_keywords.clone(),
        is_new: product.is_new,
        is_preorder: product.is_preorder,
        is_staff_pick: product.is_staff_pick,
        discount_percent: product.discount_percent.to_string(),
        display_order: product.display_order.to_string(),
        slug: product.slug.clone().unwrap_or_default(),
        seo_title: product.seo_title.clone(),
        meta_description: product.meta_description.clone(),
        track_stock: product.track_stock,
        allow_backorder: product.allow_backorder,
        low_stock_threshold: opt_int(product.low_stock_threshold),
        points_earn_percent: opt_int(product.points_earn_percent),
        weight_grams: opt_int(product.weight_grams),
        size_length_mm: opt_int(product.size_length_mm),
        size_width_mm: opt_int(product.size_width_mm),
        size_height_mm: opt_int(product.size_height_mm),
        cool_delivery: product.cool_delivery,
        mail_delivery: product.mail_delivery,
        free_shipping: product.free_shipping,
        undeliverable_regions: product.undeliverable_regions.clone(),
        lead_time_days: opt_int(product.lead_time_days),
        publish_starts_at: jst_local(product.publish_starts_at),
        publish_ends_at: jst_local(product.publish_ends_at),
        axes: matrix
            .axes
            .iter()
            .map(|a| AxisFormInput {
                id: a.axis.id.to_string(),
                name: a.axis.name.clone(),
                options: a
                    .options
                    .iter()
                    .map(|o| OptionFormInput {
                        id: o.id.to_string(),
                        value: o.value.clone(),
                    })
                    .collect(),
            })
            .collect(),
        combos: Vec::new(),
        status: product.status.clone(),
        is_event: product.is_event,
        images: attached,
        image_alts: attached_detailed
            .iter()
            .map(|(_, alt)| alt.clone())
            .collect(),
        tag_ids,
    };
    form_response(&pool, &admin, &session, &input, Some(&product), None).await
}

/// Handles the edit submit. Archived products are refused here
/// too — their form renders read-only, and a crafted POST must
/// not edit an archived record.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<FormPairs>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let product = match product_service::find_product(&pool, id).await {
        Ok(Some(product)) => product,
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("products", e),
    };
    if product.status == "archived" {
        set_flash(&session, "アーカイブ済みの商品は編集できません").await;
        return Redirect::to(&format!("/admin/products/{id}/edit")).into_response();
    }
    let mut input = input_from(&pairs);
    // Backward compatibility for posts that predate the
    // upgrade fields: the current form always submits every
    // upgrade field (empty when unset), so a field ABSENT from
    // the post belongs to an older client, and the stored
    // value is kept for it instead of being silently cleared
    // (an absent tax class or an absent stock-tracking flag
    // must never change the product by accident).
    {
        let has = |name: &str| pairs.iter().any(|(k, _)| k == name);
        if !has("tax_class_id") {
            input.tax_class_id = product.tax_class_id.to_string();
        }
        if !has("cost_yen") {
            input.cost_yen = opt_int(product.cost_yen);
        }
        if !has("sale_price_yen") {
            input.sale_price_yen = opt_int(product.sale_price_yen);
        }
        if !has("sale_starts_at") {
            input.sale_starts_at = jst_local(product.sale_starts_at);
        }
        if !has("sale_ends_at") {
            input.sale_ends_at = jst_local(product.sale_ends_at);
        }
        if !has("product_code") {
            input.product_code = product.product_code.clone().unwrap_or_default();
        }
        if !has("brand") {
            input.brand = product.brand.clone();
        }
        if !has("manufacturer") {
            input.manufacturer = product.manufacturer.clone();
        }
        if !has("country_of_origin") {
            input.country_of_origin = product.country_of_origin.clone();
        }
        if !has("spec_text") {
            input.spec_text = product.spec_text.clone();
        }
        if !has("features_text") {
            input.features_text = product.features_text.clone();
        }
        if !has("description_sp") {
            input.description_sp = product.description_sp.clone();
        }
        if !has("search_keywords") {
            input.search_keywords = product.search_keywords.clone();
        }
        if !has("tax_class_id") {
            input.is_new = product.is_new;
        }
        if !has("tax_class_id") {
            input.is_preorder = product.is_preorder;
        }
        if !has("tax_class_id") {
            input.is_staff_pick = product.is_staff_pick;
        }
        if !has("discount_percent") {
            input.discount_percent = product.discount_percent.to_string();
        }
        if !has("display_order") {
            input.display_order = product.display_order.to_string();
        }
        if !has("slug") {
            input.slug = product.slug.clone().unwrap_or_default();
        }
        if !has("seo_title") {
            input.seo_title = product.seo_title.clone();
        }
        if !has("meta_description") {
            input.meta_description = product.meta_description.clone();
        }
        if !has("tax_class_id") {
            input.track_stock = product.track_stock;
        }
        if !has("tax_class_id") {
            input.allow_backorder = product.allow_backorder;
        }
        if !has("low_stock_threshold") {
            input.low_stock_threshold = opt_int(product.low_stock_threshold);
        }
        if !has("points_earn_percent") {
            input.points_earn_percent = opt_int(product.points_earn_percent);
        }
        if !has("weight_grams") {
            input.weight_grams = opt_int(product.weight_grams);
        }
        if !has("size_length_mm") {
            input.size_length_mm = opt_int(product.size_length_mm);
        }
        if !has("size_width_mm") {
            input.size_width_mm = opt_int(product.size_width_mm);
        }
        if !has("size_height_mm") {
            input.size_height_mm = opt_int(product.size_height_mm);
        }
        if !has("tax_class_id") {
            input.cool_delivery = product.cool_delivery;
        }
        if !has("tax_class_id") {
            input.mail_delivery = product.mail_delivery;
        }
        if !has("tax_class_id") {
            input.free_shipping = product.free_shipping;
        }
        if !has("undeliverable_regions") {
            input.undeliverable_regions = product.undeliverable_regions.clone();
        }
        if !has("lead_time_days") {
            input.lead_time_days = opt_int(product.lead_time_days);
        }
        if !has("publish_starts_at") {
            input.publish_starts_at = jst_local(product.publish_starts_at);
        }
        if !has("publish_ends_at") {
            input.publish_ends_at = jst_local(product.publish_ends_at);
        }
    }
    // The tag list is the validation vocabulary: swallowing a
    // failed query would reject every submitted tag as unknown,
    // so the error propagates like the other admin queries.
    let tags = match product_service::list_tags(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    let valid_ids: Vec<i64> = tags.iter().map(|t| t.id).collect();
    // The removal protection validates against the stored
    // matrix: an ordered combination must survive the edit.
    let existing = match product_service::existing_matrix(&pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    // The category vocabulary is the managed table; the
    // product's own current category is passed so an inactive
    // one still validates as its own (never as a new choice).
    let categories = match product_service::list_category_records(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    // The product's attached images: the only non-upload
    // paths the save may keep (grandfathered built-in assets
    // and the uploads of earlier saves).
    let attached = match product_service::product_images(&pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    let (tax_classes, taken) = match tax_and_taken(&pool, Some(id)).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    match product_service::validate_product(
        &input,
        &attached,
        &valid_ids,
        &categories,
        product.category_id,
        Some(&existing),
        &tax_classes,
        &taken,
    ) {
        Ok(valid) => {
            // The edit form carries the `updated_at` it was
            // opened with as `expected` (epoch microseconds);
            // an absent field is an older client and skips the
            // staleness check, exactly like the absent upgrade
            // fields above.
            let expected = form_field(&pairs, "expected").trim().parse::<i64>().ok();
            match product_service::update_product(&pool, admin.id, id, &valid, None, expected).await
            {
                Ok(()) => {
                    set_flash(&session, "商品を更新しました").await;
                    Redirect::to(&format!("/admin/products/{id}/edit")).into_response()
                }
                Err(crate::error::AppError::Conflict(message)) => {
                    set_flash(&session, &message).await;
                    Redirect::to(&format!("/admin/products/{id}/edit")).into_response()
                }
                Err(crate::error::AppError::Validation(errors)) => {
                    // Save-time validation (e.g. the stock-ledger
                    // guard in the matrix write) is a form error
                    // like any other: re-render the edit form
                    // with the field errors and the typed input,
                    // never a 500.
                    let by_field = product_service::errors_by_field(&errors);
                    form_response(
                        &pool,
                        &admin,
                        &session,
                        &input,
                        Some(&product),
                        Some(&by_field),
                    )
                    .await
                }
                Err(e) => db_error("products", e),
            }
        }
        Err(errors) => {
            let by_field = product_service::errors_by_field(&errors);
            form_response(
                &pool,
                &admin,
                &session,
                &input,
                Some(&product),
                Some(&by_field),
            )
            .await
        }
    }
}

/// The product's stock history page: every ledger row for its
/// combinations, newest first (capped — the page is a working
/// view, not an export).
pub async fn stock_history(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let product = match product_service::find_product(&pool, id).await {
        Ok(Some(product)) => product,
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("products", e),
    };
    let rows = match crate::repositories::stock_ledger_repository::list_for_product(&pool, id, 200)
        .await
    {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    render(crate::views::admin::products::StockHistoryView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "products",
        product_id: product.id,
        product_name: product.name.clone(),
        rows,
    })
}

/// Archives one product (with its native confirm in the UI).
pub async fn archive(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match product_service::find_product(&pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("products", e),
    }
    match product_service::archive_product(&pool, admin.id, id).await {
        Ok(()) => {
            set_flash(&session, "商品をアーカイブしました").await;
            Redirect::to(&format!("/admin/products/{id}/edit")).into_response()
        }
        Err(e) => db_error("products", e),
    }
}

/// Unarchives one product, back to draft.
pub async fn unarchive(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match product_service::find_product(&pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("products", e),
    }
    match product_service::unarchive_product(&pool, admin.id, id).await {
        Ok(()) => {
            set_flash(&session, "アーカイブを解除しました（下書き）").await;
            Redirect::to(&format!("/admin/products/{id}/edit")).into_response()
        }
        Err(e) => db_error("products", e),
    }
}

/// Applies one bulk status change to the selected products.
/// The form arrives as raw pairs like the product form: the
/// selection repeats the `ids` field once per checked row.
pub async fn bulk(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<FormPairs>,
) -> Response {
    let ids: Vec<i64> = form_values(&pairs, "ids")
        .iter()
        .filter_map(|v| v.parse::<i64>().ok())
        .collect();
    let return_url = safe_return_url(&pairs, "/admin/products");
    let status = match form_field(&pairs, "action").as_str() {
        "public" => "public",
        "draft" => "draft",
        "archive" => "archived",
        _ => {
            return Redirect::to(&return_url).into_response();
        }
    };
    // Cap the selection: a crafted POST could name every product
    // in the store and hold one transaction open across all of
    // them. Past the cap only the first MAX_BULK_IDS are
    // processed, and the flash says the rest were dropped — never
    // a silent truncation.
    let (ids, truncated) = if ids.len() > MAX_BULK_IDS {
        (ids[..MAX_BULK_IDS].to_vec(), true)
    } else {
        (ids, false)
    };
    match product_service::bulk_set_status(&pool, admin.id, &ids, status).await {
        Ok((changed, skipped_no_image)) => {
            if truncated {
                set_flash(
                    &session,
                    &format!(
                        "選択が上限の{MAX_BULK_IDS}件を超えたため、先頭{MAX_BULK_IDS}件のみ処理しました"
                    ),
                )
                .await;
            } else if changed > 0 || skipped_no_image > 0 {
                set_flash(
                    &session,
                    &product_service::bulk_flash(status, changed, skipped_no_image),
                )
                .await;
            }
            Redirect::to(&return_url).into_response()
        }
        Err(e) => db_error("products", e),
    }
}

/// Builds the category select's options: every active
/// category, plus the product's own current one when it has
/// been retired (shown marked 無効， so an unrelated edit never
/// forces a category change). On a failed submit the submitted
/// ID is the selected one; on a fresh page, the product's
/// stored category.
fn category_options_for(
    categories: &[crate::models::ProductCategory],
    input: &ProductFormInput,
    product: Option<&crate::models::AdminProduct>,
) -> Vec<crate::views::admin::products::FormCategoryOption> {
    let selected = input
        .category_id
        .trim()
        .parse::<i64>()
        .ok()
        .or_else(|| product.and_then(|p| p.category_id));
    let current = product.and_then(|p| p.category_id);
    categories
        .iter()
        .filter(|c| c.is_active || current == Some(c.id))
        .map(|c| crate::views::admin::products::FormCategoryOption {
            id: c.id,
            name: c.name.clone(),
            inactive: !c.is_active,
            selected: selected == Some(c.id),
        })
        .collect()
}

/// Renders the shared product form for the given input state.
async fn form_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    input: &ProductFormInput,
    product: Option<&crate::models::AdminProduct>,
    errors: Option<&std::collections::BTreeMap<&'static str, String>>,
) -> Response {
    let tags = match product_service::list_tags(pool).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    let categories = match product_service::list_category_records(pool).await {
        Ok(v) => v,
        Err(e) => return db_error("products", e),
    };
    let category_options = category_options_for(&categories, input, product);
    let archived = product.is_some_and(|p| p.status == "archived");
    let err = |field: &str| errors.and_then(|m| m.get(field).cloned());
    // The grid's prefill data: on a fresh edit page, the stored
    // combinations (matched by option values in the editor); on
    // a failed submit the posted stocks ride along in the input
    // instead, so what the operator typed is what they see.
    let existing_combos_json = if errors.is_none() {
        match product {
            Some(p) => match product_service::combos_full(pool, p.id).await {
                Ok(combos) => crate::util::json_for_script::json_for_script(
                    &combos
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "values": c.values,
                                "stock": c.stock,
                                "price": c.price_yen,
                                "sku": c.sku,
                                "jan": c.jan_code,
                                "image": c.image_path,
                                "enabled": c.is_enabled,
                                "incoming": c.incoming_qty,
                                "incomingDate": c.incoming_date,
                            })
                        })
                        .collect::<Vec<_>>(),
                ),
                Err(e) => return db_error("products", e),
            },
            None => "[]".to_string(),
        }
    } else {
        "[]".to_string()
    };
    // The tax class options ride on today's rates; the
    // exclusive-price reference converts the current price at
    // the selected class's rate.
    let class_rates = match crate::repositories::tax_repository::class_rates_on(
        pool,
        crate::models::jst::jst_today(),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return db_error("products", crate::error::AppError::db("tax classes", e)),
    };
    let selected_tax_id = input.tax_class_id.trim().parse::<i64>().unwrap_or(1);
    let tax_class_options = class_rates
        .iter()
        .map(|cr| crate::views::admin::products::TaxClassOption {
            id: cr.class.id,
            label: format!("{}（{}%）", cr.class.name, cr.rate_percent),
            selected: cr.class.id == selected_tax_id,
            rate_percent: cr.rate_percent,
        })
        .collect::<Vec<_>>();
    let selected_rate = class_rates
        .iter()
        .find(|cr| cr.class.id == selected_tax_id)
        .map(|cr| cr.rate_percent)
        .unwrap_or(10);
    let price_excl_display = input
        .price_yen
        .trim()
        .parse::<i64>()
        .map(|incl| {
            crate::pricing::display_price(crate::models::tax::exclusive_from_inclusive(
                incl,
                selected_rate,
                crate::models::tax::TaxRounding::configured(),
            ))
        })
        .unwrap_or_default();
    let errors_map: std::collections::BTreeMap<String, String> = errors
        .map(|m| m.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
        .unwrap_or_default();
    render(ProductFormView {
        expected: product
            .map(|p| p.updated_at.timestamp_micros().to_string())
            .unwrap_or_default(),
        layout: layout_for(pool, admin, session).await,
        nav: "products",
        heading: if product.is_some() {
            "商品を編集".to_string()
        } else {
            "商品を登録".to_string()
        },
        // ストアで見る: the edit page links the product's storefront
        // page in a new tab — but only a public product has one; a
        // draft or archived product's page is the storefront's 404,
        // so the link renders inert with the reason instead of
        // sending the operator to an error page. The stored status
        // decides, not the form's unsaved select value.
        storefront_url: product.map(|p| format!("/products/{}", p.id)),
        storefront_note: match product {
            Some(p) if p.status == "draft" => {
                Some("下書きのため店舗には表示されていません".to_string())
            }
            Some(p) if p.status == "archived" => {
                Some("アーカイブのため店舗には表示されていません".to_string())
            }
            _ => None,
        },
        action: match product {
            Some(p) => format!("/admin/products/{}/edit", p.id),
            None => "/admin/products/new".to_string(),
        },
        name: input.name.clone(),
        description: input.description.clone(),
        category_options,
        price_yen: input.price_yen.clone(),
        variant_axes: input
            .axes
            .iter()
            .enumerate()
            .map(|(index, axis)| FormAxis {
                index,
                id: axis.id.clone(),
                name: axis.name.clone(),
                options: axis
                    .options
                    .iter()
                    .enumerate()
                    .map(|(option_index, option)| FormOption {
                        index: option_index,
                        id: option.id.clone(),
                        value: option.value.clone(),
                    })
                    .collect(),
            })
            .collect(),
        existing_combos_json,
        grid_combos_json: crate::util::json_for_script::json_for_script(
            &input
                .combos
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "stock": c.stock,
                        "price": c.price_yen,
                        "sku": c.sku,
                        "jan": c.jan_code,
                        "image": c.image_path,
                        "enabled": c.is_enabled,
                        "incoming": c.incoming_qty,
                        "incomingDate": c.incoming_date,
                    })
                })
                .collect::<Vec<_>>(),
        ),
        errors_map,
        tax_class_options,
        price_mode: input.price_mode.clone(),
        price_excl_yen: input.price_excl_yen.clone(),
        price_excl_display,
        tax_rounding: match crate::models::tax::TaxRounding::configured() {
            crate::models::tax::TaxRounding::Floor => "floor",
            crate::models::tax::TaxRounding::Round => "round",
            crate::models::tax::TaxRounding::Ceil => "ceil",
        }
        .to_string(),
        cost_yen: input.cost_yen.clone(),
        tax_class_id: input.tax_class_id.clone(),
        sale_price_yen: input.sale_price_yen.clone(),
        sale_starts_at: input.sale_starts_at.clone(),
        sale_ends_at: input.sale_ends_at.clone(),
        product_code: input.product_code.clone(),
        brand: input.brand.clone(),
        manufacturer: input.manufacturer.clone(),
        country_of_origin: input.country_of_origin.clone(),
        spec_text: input.spec_text.clone(),
        features_text: input.features_text.clone(),
        description_sp: input.description_sp.clone(),
        search_keywords: input.search_keywords.clone(),
        is_new: input.is_new,
        is_preorder: input.is_preorder,
        is_staff_pick: input.is_staff_pick,
        discount_percent: input.discount_percent.clone(),
        display_order: input.display_order.clone(),
        slug: input.slug.clone(),
        seo_title: input.seo_title.clone(),
        meta_description: input.meta_description.clone(),
        track_stock: input.track_stock,
        allow_backorder: input.allow_backorder,
        low_stock_threshold: input.low_stock_threshold.clone(),
        points_earn_percent: input.points_earn_percent.clone(),
        weight_grams: input.weight_grams.clone(),
        size_length_mm: input.size_length_mm.clone(),
        size_width_mm: input.size_width_mm.clone(),
        size_height_mm: input.size_height_mm.clone(),
        cool_delivery: input.cool_delivery,
        mail_delivery: input.mail_delivery,
        free_shipping: input.free_shipping,
        undeliverable_regions: input.undeliverable_regions.clone(),
        lead_time_days: input.lead_time_days.clone(),
        publish_starts_at: input.publish_starts_at.clone(),
        publish_ends_at: input.publish_ends_at.clone(),
        stock_history_url: match product {
            Some(p) => format!("/admin/products/{}/stock-history", p.id),
            None => String::new(),
        },
        restock_summary: match product {
            Some(p) => match crate::services::restock_service::counts_for_product(pool, p.id).await
            {
                Ok((waiting, sent)) if waiting + sent > 0 => Some(format!(
                    "再入荷通知：登録待ち {waiting} 件・送信済み {sent} 件"
                )),
                Ok(_) => None,
                Err(e) => {
                    tracing::error!(product_id = p.id, "restock counts failed: {e}");
                    None
                }
            },
            None => None,
        },
        status: input.status.clone(),
        is_event: input.is_event,
        images: input
            .images
            .iter()
            .enumerate()
            .map(|(index, path)| FormImage {
                path: path.clone(),
                alt: input.image_alts.get(index).cloned().unwrap_or_default(),
            })
            .collect(),
        tags: tags
            .iter()
            .map(|t| TagCheck {
                id: t.id,
                label: t.label.clone(),
                checked: input.tag_ids.contains(&t.id),
            })
            .collect(),
        readonly: archived,
        status_note: if archived {
            "この商品はアーカイブ済みです。編集するには先にアーカイブを解除してください。"
                .to_string()
        } else {
            String::new()
        },
        unarchive_action: match product {
            Some(p) if archived => format!("/admin/products/{}/unarchive", p.id),
            _ => String::new(),
        },
        archive_action: match product {
            Some(p) if !archived => format!("/admin/products/{}/archive", p.id),
            _ => String::new(),
        },
        name_error: err("name"),
        description_error: err("description"),
        category_error: err("category"),
        price_error: err("price_yen"),
        variants_error: err("variants"),
        image_error: err("image"),
    })
}

// ------------------------------------------------------------------
// CSV import/export (product CSV service): download, upload,
// preview, apply, discard. All POSTs ride the admin surface
// stack like every other admin form.
// ------------------------------------------------------------------

/// Body ceiling for the CSV upload routes: the configured
/// file ceiling (10 MiB by default) plus room for the
/// multipart framing. The handler's check against
/// `csv_max_file_bytes` reports the real limit.
const CSV_BODY_LIMIT: usize = 12 * 1024 * 1024;

/// CSV export download: the products the list's current tab
/// and filters match, unpaginated, one row per variant.
pub async fn csv_export(
    State(pool): State<PgPool>,
    CurrentAdmin(_admin): CurrentAdmin,
    Query(query): Query<ListQuery>,
) -> Response {
    let tab = match query.tab.as_deref() {
        Some("public") => "public",
        Some("draft") => "draft",
        Some("archived") => "archived",
        _ => "all",
    };
    let sort = query
        .sort
        .as_deref()
        .filter(|s| SORTS.iter().any(|(v, _)| v == s))
        .unwrap_or("new");
    let q = query.q.clone().unwrap_or_default();
    let keyword = crate::util::like_pattern::like_pattern(&q);
    let category = query.category.clone().filter(|c| !c.trim().is_empty());
    let tag_id = query.tag.as_deref().and_then(|t| t.parse::<i64>().ok());
    let query = product_service::AdminProductQuery {
        tab,
        keyword,
        category,
        tag_id,
        sort,
        limit: i64::MAX,
        offset: 0,
    };
    match product_csv_service::export_csv(&pool, &query).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"products.csv\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => db_error("product csv export", e),
    }
}

/// The CSV upload form page.
pub async fn csv_import_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    render(ProductCsvImportView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "products",
    })
}

/// Receives one CSV upload (multipart, field `file`):
/// parses and validates it into a stored preview batch,
/// then redirects to the preview page — including for a
/// file whose problems are all content problems, which the
/// preview page explains line by line.
pub async fn csv_upload(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    mut multipart: Multipart,
) -> Response {
    let mut file: Option<(String, Vec<u8>)> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let name = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "import.csv".to_string());
        match field.bytes().await {
            Ok(bytes) => file = Some((name, bytes.to_vec())),
            Err(_) => break,
        }
    }
    let Some((name, bytes)) = file.filter(|(_, bytes)| !bytes.is_empty()) else {
        set_flash(&session, CSV_FILE_REQUIRED_MESSAGE).await;
        return Redirect::to("/admin/products/csv-import").into_response();
    };
    if bytes.len() as i64 > CONFIG.csv_max_file_bytes {
        set_flash(&session, CSV_FILE_TOO_LARGE_MESSAGE).await;
        return Redirect::to("/admin/products/csv-import").into_response();
    }
    match product_csv_service::store_upload(&pool, admin.id, &name, &bytes).await {
        Ok(id) => Redirect::to(&format!("/admin/products/csv-import/{id}")).into_response(),
        Err(e) => db_error("product csv upload", e),
    }
}

/// The preview page for one stored batch: the validation
/// report's counts and per-line errors, with the apply and
/// discard actions while the batch is still a preview.
pub async fn csv_preview(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let batch = match csv_import_batch_repository::find_batch(&pool, id).await {
        Ok(Some(batch)) if batch.entity == "product" => batch,
        Ok(_) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("product csv preview", e),
    };
    let report = BatchReport::from_json(&batch.report);
    render(ProductCsvPreviewView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "products",
        batch_id: batch.id,
        file_name: batch.file_name,
        status: batch.status,
        new_count: report.new_count,
        update_count: report.update_count,
        error_count: report.error_count(),
        errors: report.errors,
    })
}

/// Applies one preview batch through the product save path.
pub async fn csv_apply(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match product_csv_service::apply_batch(&pool, admin.id, id).await {
        Ok((created, updated)) => {
            set_flash(&session, &csv_import_applied_message(created, updated)).await;
            Redirect::to("/admin/products").into_response()
        }
        Err(crate::error::AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("product csv apply", e),
    }
}

/// Discards one preview batch; no product is touched.
pub async fn csv_discard(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match product_csv_service::discard_batch(&pool, id).await {
        Ok(()) => {
            set_flash(&session, CSV_IMPORT_DISCARDED_NOTICE).await;
            Redirect::to("/admin/products").into_response()
        }
        Err(crate::error::AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("product csv discard", e),
    }
}
