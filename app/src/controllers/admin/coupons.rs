//! Coupon definitions and usage.
//!
//! The definitions list is the section's home (Coupon
//! Admin spec): definitions are created and edited here,
//! with redemption locks on the fields that would rewrite
//! placed-order economics. The usage list (who holds which
//! copy, whether it was used) lives under /usage.

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, pagination, record_not_found, render, set_flash};
use crate::config::CONFIG;
use crate::services::admin::coupon_service;
use crate::session::CurrentAdmin;
use crate::views::admin::Page;
use crate::views::admin::coupons::{CouponListView, CouponRow};

/// Routes: the definitions list is the section's home;
/// the usage list lives under /usage; definitions are
/// created and edited like products (Coupon Admin
/// spec §6).
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/coupons", get(definitions).post(create))
        .route("/admin/coupons/usage", get(list))
        .route("/admin/coupons/new", get(new_form))
        .route("/admin/coupons/{id}", get(edit_form).post(update))
        .route(
            "/admin/coupons/{id}/deactivate",
            get(deactivate_form).post(deactivate),
        )
        .route(
            "/admin/coupons/{id}/activate",
            axum::routing::post(activate),
        )
}

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Page number.
    page: Option<String>,
    /// Definition filter (the definitions list's
    /// 利用状況 link).
    code: Option<String>,
}

/// The coupon usage page.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let (rows, total) =
        match coupon_service::list_admin(&pool, limit, offset, query.code.as_deref()).await {
            Ok(v) => v,
            Err(e) => return db_error("coupons", e),
        };
    let coupons = rows
        .into_iter()
        .map(|row| {
            // The using order's number is computed from the row
            // itself: the list query carries the order's ID and
            // placement instant, so this page costs one query total.
            let used_order_number = coupon_service::used_order_number(&row);
            CouponRow {
                code: row.code,
                description: row.description,
                holder: row.holder_name.unwrap_or_default(),
                issued: crate::models::jst::jst_date_label(row.issued_at),
                state_label: if row.used_at.is_some() {
                    "使用済み"
                } else {
                    "未使用"
                },
                used_order_id: row.used_order_id,
                used_order_number,
            }
        })
        .collect();
    render(CouponListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "coupons",
        coupons,
        page: Page::new(
            page_no,
            total,
            limit,
            match &query.code {
                Some(code) => Page::base_with("/admin/coupons/usage", &format!("code={code}")),
                None => Page::base_with("/admin/coupons/usage", ""),
            },
        ),
    })
}

// ---- Coupon definition management (Coupon Admin spec §6) ----

use crate::services::admin::coupon_service::{
    CouponDefinitionInput, CouponWriteError, create_definition, set_definition_active,
    update_definition,
};
use crate::views::admin::coupons::{
    CouponDeactivateView, CouponDefinitionsView, CouponFormErrors, CouponFormValues,
    CouponFormView, DefinitionRow, DefinitionTab,
};

/// Definitions list query: the status tab.
#[derive(Deserialize, Default)]
pub struct DefinitionsQuery {
    /// Tab: active / inactive (absent = all).
    status: Option<String>,
}

/// Formats an optional UTC instant as a JST
/// `datetime-local` input value.
fn input_value(at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    let jst = crate::models::jst::jst();
    at.map(|t| t.with_timezone(&jst).format("%Y-%m-%dT%H:%M").to_string())
        .unwrap_or_default()
}

/// The window label for the definitions list.
fn window_label(
    starts_at: Option<chrono::DateTime<chrono::Utc>>,
    ends_at: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    let fmt = crate::models::jst::jst_datetime_label;
    match (starts_at, ends_at) {
        (None, None) => "常時".to_string(),
        (Some(s), None) => format!("{} 〜", fmt(s)),
        (None, Some(e)) => format!("〜 {}", fmt(e)),
        (Some(s), Some(e)) => format!("{} 〜 {}", fmt(s), fmt(e)),
    }
}

/// `GET /admin/coupons`: the definitions list with
/// status tabs and redemption counts.
pub async fn definitions(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<DefinitionsQuery>,
) -> Response {
    let all = match coupon_service::list_definitions(&pool).await {
        Ok(rows) => rows,
        Err(e) => return db_error("coupon definitions", e),
    };
    let tab = query.status.as_deref().unwrap_or("all");
    let count = |want: Option<bool>| -> i64 {
        all.iter()
            .filter(|r| want.is_none() || want == Some(r.active))
            .count() as i64
    };
    let tabs = vec![
        DefinitionTab {
            label: "すべて".to_string(),
            url: "/admin/coupons".to_string(),
            count: count(None),
            active: tab == "all",
        },
        DefinitionTab {
            label: "有効".to_string(),
            url: "/admin/coupons?status=active".to_string(),
            count: count(Some(true)),
            active: tab == "active",
        },
        DefinitionTab {
            label: "無効".to_string(),
            url: "/admin/coupons?status=inactive".to_string(),
            count: count(Some(false)),
            active: tab == "inactive",
        },
    ];
    let want = match tab {
        "active" => Some(true),
        "inactive" => Some(false),
        _ => None,
    };
    let rows = all
        .into_iter()
        .filter(|r| want.is_none() || want == Some(r.active))
        .map(|r| DefinitionRow {
            id: r.id,
            code: r.code,
            type_display: match r.discount_type.as_str() {
                "percent" => format!("定率 {}%割引", r.percent.unwrap_or(0)),
                "free_shipping" => "送料無料".to_string(),
                _ => format!("定額 {}割引", crate::pricing::display_price(r.discount_yen)),
            },
            scope_display: match r.scope.as_str() {
                "product" => format!("商品：{}", r.target_label.clone().unwrap_or_default()),
                "variant" => {
                    format!(
                        "バリエーション：{}",
                        r.target_label.clone().unwrap_or_default()
                    )
                }
                _ => "注文全体".to_string(),
            },
            automatic: r.is_automatic,
            discount_display: crate::pricing::display_price(r.discount_yen),
            min_display: if r.min_subtotal_yen > 0 {
                crate::pricing::display_price(r.min_subtotal_yen)
            } else {
                "ー".to_string()
            },
            window_label: window_label(r.starts_at, r.ends_at),
            description: r.description,
            active: r.active,
            band_featured: r.band_featured,
            redemptions: r.redemptions,
        })
        .collect();
    render(CouponDefinitionsView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "coupons",
        tabs,
        rows,
    })
}

/// The definition form as posted.
#[derive(Deserialize)]
pub struct DefinitionForm {
    /// Code.
    code: String,
    /// Value kind as selected (absent = fixed, the pre-pack
    /// shape older posts carry).
    #[serde(default)]
    discount_type: String,
    /// Discount as typed.
    discount_yen: String,
    /// Percent as typed.
    #[serde(default)]
    percent: String,
    /// Scope kind as selected (absent = order).
    #[serde(default)]
    scope: String,
    /// Scoped product id as selected.
    #[serde(default)]
    product_id: String,
    /// Scoped variant id as selected.
    #[serde(default)]
    variant_id: String,
    /// Total cap as typed.
    #[serde(default)]
    max_uses_total: String,
    /// Per-customer cap as typed.
    #[serde(default)]
    max_uses_per_customer: String,
    /// Automatic checkbox (present when checked).
    #[serde(default)]
    is_automatic: Option<String>,
    /// Minimum subtotal as typed.
    min_subtotal_yen: String,
    /// Message.
    description: String,
    /// Window start.
    #[serde(default)]
    starts_at: String,
    /// Window end.
    #[serde(default)]
    ends_at: String,
    /// Band checkbox (present when checked).
    #[serde(default)]
    band_featured: Option<String>,
    /// Active checkbox (create only).
    #[serde(default)]
    active: Option<String>,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl DefinitionForm {
    /// Converts to the service input.
    fn to_input(&self) -> CouponDefinitionInput {
        CouponDefinitionInput {
            code: self.code.clone(),
            discount_type: self.discount_type.clone(),
            discount_yen: self.discount_yen.clone(),
            percent: self.percent.clone(),
            scope: self.scope.clone(),
            product_id: self.product_id.clone(),
            variant_id: self.variant_id.clone(),
            max_uses_total: self.max_uses_total.clone(),
            max_uses_per_customer: self.max_uses_per_customer.clone(),
            is_automatic: self.is_automatic.is_some(),
            min_subtotal_yen: self.min_subtotal_yen.clone(),
            description: self.description.clone(),
            starts_at: self.starts_at.clone(),
            ends_at: self.ends_at.clone(),
            band_featured: self.band_featured.is_some(),
            active: self.active.is_some(),
        }
    }

    /// Converts to preserved form values.
    fn to_values(&self) -> CouponFormValues {
        CouponFormValues {
            code: self.code.clone(),
            discount_type: self.discount_type.clone(),
            discount_yen: self.discount_yen.clone(),
            percent: self.percent.clone(),
            scope: self.scope.clone(),
            product_id: self.product_id.clone(),
            variant_id: self.variant_id.clone(),
            max_uses_total: self.max_uses_total.clone(),
            max_uses_per_customer: self.max_uses_per_customer.clone(),
            is_automatic: self.is_automatic.is_some(),
            min_subtotal_yen: self.min_subtotal_yen.clone(),
            description: self.description.clone(),
            starts_at: self.starts_at.clone(),
            ends_at: self.ends_at.clone(),
            band_featured: self.band_featured.is_some(),
            active: self.active.is_some(),
        }
    }
}

/// The form page's fixed context (everything except
/// the values and errors that change per render).
struct FormContext {
    /// Page heading.
    heading: &'static str,
    /// Where the form posts.
    form_action: String,
    /// Whether the create-only active checkbox shows.
    show_active: bool,
    /// The §5.1 locks, on the edit form.
    locks: Option<(bool, bool)>,
    /// The definition id on the edit form.
    definition_id: Option<i64>,
    /// The current active state on the edit form.
    current_active: Option<bool>,
}

/// Renders the form page.
/// The form-ticket scope of one coupon definition.
fn coupon_scope(id: i64) -> String {
    format!("coupon:{id}")
}

/// Mints an edit ticket for the definition at its current
/// stamp, read through the coupon service (the
/// `CouponDefinition` row struct does not carry
/// `updated_at`).
async fn coupon_ticket(pool: &PgPool, session: &Session, id: i64) -> String {
    let stamp = coupon_service::stamp(pool, id).await;
    crate::services::form_ticket_service::issue(session, &coupon_scope(id), stamp).await
}

async fn form_response(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    ctx: FormContext,
    values: CouponFormValues,
    errors: CouponFormErrors,
    form_ticket: String,
) -> Response {
    let (code_locked, money_locked) = ctx.locks.unwrap_or((false, false));
    let (product_rows, variant_rows) = coupon_service::form_options(pool).await;
    // On the variant edit form the product picker only
    // filters the variant list, so it opens on the chosen
    // variant's parent product.
    let mut product_selected = values.product_id.clone();
    if product_selected.is_empty()
        && let Some((_, _, parent)) = variant_rows
            .iter()
            .find(|(id, _, _)| id.to_string() == values.variant_id)
    {
        product_selected = parent.to_string();
    }
    let products = product_rows
        .into_iter()
        .map(|(id, label)| {
            let id = id.to_string();
            crate::views::admin::coupons::CouponTargetOption {
                selected: id == product_selected,
                id,
                label,
                parent_id: String::new(),
            }
        })
        .collect::<Vec<_>>();
    let variants = variant_rows
        .into_iter()
        .map(|(id, label, parent)| {
            let id = id.to_string();
            crate::views::admin::coupons::CouponTargetOption {
                selected: id == values.variant_id,
                id,
                label,
                parent_id: parent.to_string(),
            }
        })
        .collect::<Vec<_>>();
    render(CouponFormView {
        layout: layout_for(pool, admin, session).await,
        nav: "coupons",
        heading: ctx.heading.to_string(),
        form_action: ctx.form_action,
        values,
        errors,
        show_active: ctx.show_active,
        code_locked,
        money_locked,
        products,
        variants,
        definition_id: ctx.definition_id,
        current_active: ctx.current_active,
        form_ticket,
    })
}

/// `GET /admin/coupons/new`: the blank create form.
pub async fn new_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let values = CouponFormValues {
        active: true,
        discount_type: "fixed".to_string(),
        scope: "order".to_string(),
        max_uses_per_customer: "1".to_string(),
        ..Default::default()
    };
    form_response(
        &pool,
        &admin,
        &session,
        FormContext {
            heading: "クーポンを作成",
            form_action: "/admin/coupons".to_string(),
            show_active: true,
            locks: None,
            definition_id: None,
            current_active: None,
        },
        values,
        CouponFormErrors::default(),
        String::new(),
    )
    .await
}

/// `POST /admin/coupons`: creates a definition.
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<DefinitionForm>,
) -> Response {
    match create_definition(&pool, admin.id, &form.to_input()).await {
        Ok(id) => {
            set_flash(&session, "クーポンを作成しました。").await;
            Redirect::to(&format!("/admin/coupons/{id}")).into_response()
        }
        Err(CouponWriteError::Validation(errors)) => {
            form_response(
                &pool,
                &admin,
                &session,
                FormContext {
                    heading: "クーポンを作成",
                    form_action: "/admin/coupons".to_string(),
                    show_active: true,
                    locks: None,
                    definition_id: None,
                    current_active: None,
                },
                form.to_values(),
                CouponFormErrors::from_field_errors(&errors),
                String::new(),
            )
            .await
        }
        Err(CouponWriteError::NotFound) => Redirect::to("/admin/coupons").into_response(),
        // Stale comes only from the update service, never from create.
        Err(CouponWriteError::Stale) => Redirect::to("/admin/coupons").into_response(),
        Err(CouponWriteError::Db(e)) => db_error("coupon create", e),
    }
}

/// `GET /admin/coupons/{id}`: the edit form, with any
/// §5.1 locks shown.
pub async fn edit_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let Some(definition) = coupon_service::definition(&pool, id).await else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let locks = coupon_service::field_locks(&pool, &definition).await;
    let values = CouponFormValues {
        code: definition.code.clone(),
        discount_type: definition.discount_type.clone(),
        discount_yen: definition.discount_yen.to_string(),
        percent: definition
            .percent
            .map(|v| v.to_string())
            .unwrap_or_default(),
        scope: definition.scope.clone(),
        product_id: definition
            .product_id
            .map(|v| v.to_string())
            .unwrap_or_default(),
        variant_id: definition
            .variant_id
            .map(|v| v.to_string())
            .unwrap_or_default(),
        max_uses_total: definition
            .max_uses_total
            .map(|v| v.to_string())
            .unwrap_or_default(),
        max_uses_per_customer: definition
            .max_uses_per_customer
            .map(|v| v.to_string())
            .unwrap_or_default(),
        is_automatic: definition.is_automatic,
        min_subtotal_yen: definition.min_subtotal_yen.to_string(),
        description: definition.description.clone(),
        starts_at: input_value(definition.starts_at),
        ends_at: input_value(definition.ends_at),
        band_featured: definition.band_featured,
        active: definition.active,
    };
    form_response(
        &pool,
        &admin,
        &session,
        FormContext {
            heading: "クーポンを編集",
            form_action: format!("/admin/coupons/{id}"),
            show_active: false,
            locks: Some(locks),
            definition_id: Some(id),
            current_active: Some(definition.active),
        },
        values,
        CouponFormErrors::default(),
        coupon_ticket(&pool, &session, id).await,
    )
    .await
}

/// `POST /admin/coupons/{id}`: saves an edit.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<DefinitionForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    // Spend the form ticket before the save: a submit
    // without a valid ticket was not rendered from the
    // current record, so it is refused as stale. A ticket
    // whose stamp has gone stale is refused by the
    // service's check instead.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &coupon_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/coupons/{id}")).into_response();
    };
    match update_definition(&pool, admin.id, id, &form.to_input(), expected).await {
        Ok(()) => {
            set_flash(&session, "クーポンを保存しました。").await;
            Redirect::to(&format!("/admin/coupons/{id}")).into_response()
        }
        Err(CouponWriteError::Validation(errors)) => {
            let definition = coupon_service::definition(&pool, id).await;
            let locks = match &definition {
                Some(d) => Some(coupon_service::field_locks(&pool, d).await),
                None => None,
            };
            form_response(
                &pool,
                &admin,
                &session,
                FormContext {
                    heading: "クーポンを編集",
                    form_action: format!("/admin/coupons/{id}"),
                    show_active: false,
                    locks,
                    definition_id: Some(id),
                    current_active: definition.map(|d| d.active),
                },
                form.to_values(),
                CouponFormErrors::from_field_errors(&errors),
                coupon_ticket(&pool, &session, id).await,
            )
            .await
        }
        Err(CouponWriteError::Stale) => {
            set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
            Redirect::to(&format!("/admin/coupons/{id}")).into_response()
        }
        Err(CouponWriteError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(CouponWriteError::Db(e)) => db_error("coupon update", e),
    }
}

/// `GET /admin/coupons/{id}/deactivate`: the confirm
/// page, naming the unredeemed-holder count (§5.2).
pub async fn deactivate_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let Some(definition) = coupon_service::definition(&pool, id).await else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let holders = coupon_service::unredeemed_holders(&pool, &definition.code).await;
    render(CouponDeactivateView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "coupons",
        id,
        code: definition.code,
        holders,
    })
}

/// `POST /admin/coupons/{id}/deactivate`.
pub async fn deactivate(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match set_definition_active(&pool, admin.id, id, false).await {
        Ok(()) => {
            set_flash(&session, "クーポンを無効化しました。").await;
            Redirect::to(&format!("/admin/coupons/{id}")).into_response()
        }
        Err(CouponWriteError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(CouponWriteError::Validation(_)) => Redirect::to("/admin/coupons").into_response(),
        Err(CouponWriteError::Stale) => Redirect::to("/admin/coupons").into_response(),
        Err(CouponWriteError::Db(e)) => db_error("coupon deactivate", e),
    }
}

/// `POST /admin/coupons/{id}/activate`.
pub async fn activate(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match set_definition_active(&pool, admin.id, id, true).await {
        Ok(()) => {
            set_flash(&session, "クーポンを有効化しました。").await;
            Redirect::to(&format!("/admin/coupons/{id}")).into_response()
        }
        Err(CouponWriteError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(CouponWriteError::Validation(_)) => Redirect::to("/admin/coupons").into_response(),
        Err(CouponWriteError::Stale) => Redirect::to("/admin/coupons").into_response(),
        Err(CouponWriteError::Db(e)) => db_error("coupon activate", e),
    }
}
