//! Admin order management.
//!
//! List with status filter chips (live counts) and order-number
//! search; detail with the delivery block, snapshot lines, totals,
//! and the fulfilment forms. The status form offers only the legal
//! next states; cancellation is its own form with a native confirm
//! and a stock-restore note. Every transition is re-checked in
//! [`crate::services::admin::order_service`] — a hand-crafted POST
//! is refused with an error, never applied.

use axum::{
    Form, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, pagination, record_not_found, render, set_flash};
use crate::config::CONFIG;
use crate::messages::{
    ADMIN_PROCESSING_FAILED_MESSAGE, BULK_STATUS_INVALID_MESSAGE,
    CARRIER_SELECTION_INVALID_MESSAGE, CSV_FILE_REQUIRED_MESSAGE, CSV_FILE_TOO_LARGE_MESSAGE,
    NO_ORDERS_SELECTED_MESSAGE, ORDER_STATUS_CHANGE_DENIED_MESSAGE, PARTIAL_SHIPMENT_UNIT_MESSAGE,
    PAYMENT_INFO_MISSING_MESSAGE, SHIPMENT_BLOCKED_UNPAID_NOTICE,
};
use crate::models::order::{admin_status_label, legal_next_statuses, order_number_for};
use crate::models::payment::{payment_badge_class_for_parts, payment_status_label_for_parts};
use crate::models::{Payment, PaymentMethod};
use crate::pricing;
use crate::repositories::payment_repository;
use crate::services::admin::audit_query_service;
use crate::services::admin::order_service::{self, StatusChangeError};
use crate::services::payment_service::{self, RefundError};
use crate::session::CurrentAdmin;
use crate::views::admin::audit::action_label;
use crate::views::admin::orders::{
    AdminPaymentBlock, BulkCarrierOption, BulkConfirmRow, BulkConfirmView, NextStatus, OrderChip,
    OrderDetailView, OrderHistoryRow, OrderListView, OrderRow, PackingSlip, PackingSlipsView,
    PaymentHistoryRow, TrackingImportRow, TrackingImportView,
};
use crate::views::admin::{Page, filter_query};

/// Routes for the order surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/orders", get(list))
        .route("/admin/orders/bulk-status/preview", post(bulk_preview))
        .route("/admin/orders/bulk-status", post(bulk_execute))
        .route("/admin/orders/label-csv", post(label_csv))
        .route("/admin/orders/packing-slips", get(packing_slips))
        .route(
            "/admin/orders/tracking-import",
            post(tracking_import).layer(DefaultBodyLimit::max(CSV_BODY_LIMIT)),
        )
        .route("/admin/orders/{id}", get(detail))
        .route("/admin/orders/{id}/status", post(change_status))
        .route(
            "/admin/orders/{id}/shipping-mail/resend",
            post(resend_shipping_mail),
        )
        .route("/admin/orders/{id}/cancel", post(cancel))
        .route("/admin/orders/{id}/refund", post(refund))
        .route("/admin/orders/{id}/shipments", post(create_shipment))
        .route(
            "/admin/orders/{id}/shipments/{sid}/ship",
            post(ship_shipment),
        )
        .route("/admin/orders/{id}/payment-sync", post(payment_sync))
}

/// The status filter options, in order: all, the four flow
/// states, cancelled, then the operator's `unshipped` shortcut.
const STATUS_FILTERS: [(&str, &str); 7] = [
    ("all", "すべて"),
    ("received", "受付済み"),
    ("preparing", "準備中"),
    ("shipped", "発送済み"),
    ("delivered", "配達完了"),
    ("cancelled", "キャンセル"),
    ("unshipped", "未発送"),
];

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Status filter value.
    status: Option<String>,
    /// Order-number search text.
    q: Option<String>,
    /// Page number.
    page: Option<String>,
    /// Payment filter value: `awaiting` = 入金待ち KOMOJU
    /// orders; `paid`, `refunded` and `failed` select the
    /// matching payment states.
    payment: Option<String>,
    /// Placed-on or after this JST date (`YYYY-MM-DD`).
    from: Option<String>,
    /// Placed-on or before this JST date (`YYYY-MM-DD`).
    to: Option<String>,
    /// Customer name fragment: matches the member name or the
    /// delivery recipient name.
    customer: Option<String>,
}

/// The order list page.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    let status = query
        .status
        .as_deref()
        .filter(|s| STATUS_FILTERS.iter().any(|(v, _)| v == s))
        .unwrap_or("all")
        .to_string();
    let q = query.q.clone().unwrap_or_default();
    // LIKE patterns go through the shared escaper, so a `%` or
    // `_` in a typed fragment matches literally.
    let number = crate::util::like_pattern::like_pattern(q.trim());
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let status_param = if status == "all" {
        None
    } else {
        Some(status.as_str())
    };
    // The payment filter keeps only the known state groups;
    // anything else is no filter at all.
    let payment_value = query
        .payment
        .as_deref()
        .filter(|v| matches!(*v, "awaiting" | "paid" | "refunded" | "failed"))
        .unwrap_or_default();
    let payment_filter = (!payment_value.is_empty()).then_some(payment_value);
    let awaiting = payment_filter == Some("awaiting");
    let from_text = query.from.as_deref().unwrap_or("").trim().to_string();
    let to_text = query.to.as_deref().unwrap_or("").trim().to_string();
    let customer_text = query.customer.as_deref().unwrap_or("").trim().to_string();
    // The date inputs name JST calendar days; the repository
    // compares UTC instants, half-open: [from, to + 1 day).
    let placed_from = jst_day_start(&from_text);
    let placed_to = jst_day_start(&to_text).map(|start| start + chrono::Duration::days(1));
    let customer = crate::util::like_pattern::like_pattern(&customer_text);
    let (rows, total) = match order_service::list_orders(
        &pool,
        status_param,
        number,
        payment_filter,
        placed_from,
        placed_to,
        customer,
        limit,
        offset,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return db_error("orders", e),
    };
    // The chips' counts are page data from the database like
    // any other figure here: a failed query is a 500, never
    // zeroed counts rendered as if real (the products rule).
    let counts = match order_service::status_counts(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("orders", e),
    };
    let count_of = |key: &str| -> i64 {
        counts
            .iter()
            .filter(|(s, _)| s == key)
            .map(|(_, c)| *c)
            .sum()
    };
    // The operator's `unshipped` shortcut is received + preparing.
    let received = count_of("received");
    let chip_count = |key: &str| -> i64 {
        match key {
            "all" => counts.iter().map(|(_, c)| *c).sum(),
            "unshipped" => received + count_of("preparing"),
            "received" => received,
            other => count_of(other),
        }
    };
    let mut chips: Vec<OrderChip> = STATUS_FILTERS
        .iter()
        .map(|(v, l)| {
            let qs = filter_query(&[
                ("status", (*v != "all").then_some(*v)),
                ("q", (!q.trim().is_empty()).then_some(q.trim())),
                (
                    "from",
                    (!from_text.is_empty()).then_some(from_text.as_str()),
                ),
                ("to", (!to_text.is_empty()).then_some(to_text.as_str())),
                (
                    "customer",
                    (!customer_text.is_empty()).then_some(customer_text.as_str()),
                ),
                (
                    "payment",
                    (!payment_value.is_empty()).then_some(payment_value),
                ),
            ]);
            OrderChip {
                url: if qs.is_empty() {
                    "/admin/orders".to_string()
                } else {
                    format!("/admin/orders?{qs}")
                },
                label: l,
                count: chip_count(v),
                active: status == *v && !awaiting,
            }
        })
        .collect();
    // The payment chip: orders whose KOMOJU money has not
    // arrived — the queue that needs watching, not shipping.
    let pending_count = match order_service::pending_payment_count(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("orders", e),
    };
    let payment_qs = filter_query(&[
        ("payment", Some("awaiting")),
        ("q", (!q.trim().is_empty()).then_some(q.trim())),
        (
            "from",
            (!from_text.is_empty()).then_some(from_text.as_str()),
        ),
        ("to", (!to_text.is_empty()).then_some(to_text.as_str())),
        (
            "customer",
            (!customer_text.is_empty()).then_some(customer_text.as_str()),
        ),
    ]);
    chips.push(OrderChip {
        url: format!("/admin/orders?{payment_qs}"),
        label: "入金待ち",
        count: pending_count,
        active: awaiting,
    });
    let base = Page::base_with(
        "/admin/orders",
        &filter_query(&[
            ("status", (status != "all").then_some(status.as_str())),
            ("q", (!q.trim().is_empty()).then_some(q.trim())),
            (
                "payment",
                (!payment_value.is_empty()).then_some(payment_value),
            ),
            (
                "from",
                (!from_text.is_empty()).then_some(from_text.as_str()),
            ),
            ("to", (!to_text.is_empty()).then_some(to_text.as_str())),
            (
                "customer",
                (!customer_text.is_empty()).then_some(customer_text.as_str()),
            ),
        ]),
    );
    render(OrderListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "orders",
        return_url: base.clone(),
        chips,
        status,
        q,
        from: from_text,
        to: to_text,
        customer: customer_text,
        payment: payment_value.to_string(),
        orders: rows
            .iter()
            .map(|o| OrderRow {
                id: o.id,
                number: order_number_for(o.id, o.created_at),
                buyer: o
                    .member_name
                    .clone()
                    .unwrap_or_else(|| "ゲスト".to_string()),
                member_id: o.user_id,
                item_count: o.item_count,
                payment_label: payment_status_label_for_parts(
                    &o.status,
                    o.payment_provider.as_deref(),
                    o.payment_status.as_deref(),
                ),
                payment_badge_class: payment_badge_class_for_parts(
                    &o.status,
                    o.payment_provider.as_deref(),
                    o.payment_status.as_deref(),
                ),
                cancelled: o.status == "cancelled",
                total: pricing::display_price(o.total_yen),
                status_label: admin_status_label(&o.status),
                status: o.status.clone(),
                placed: crate::models::jst::jst_datetime_label(o.created_at),
                agent_placed: o.placed_via == crate::models::order::PlacedVia::Agent.as_str(),
            })
            .collect(),
        page: Page::new(page_no, total, limit, base),
        label_carriers: {
            let settings = crate::services::shop_settings_service::load(&pool).await;
            crate::models::carrier::CARRIERS
                .iter()
                .filter(|c| {
                    !settings.carrier_disabled(c.code)
                        && crate::services::admin::shipping_csv_service::LABEL_CARRIER_CODES
                            .contains(&c.code)
                })
                .map(|c| BulkCarrierOption {
                    code: c.code,
                    label: c.label,
                })
                .collect()
        },
    })
}

/// The order detail page.
pub async fn detail(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    detail_response(&pool, &admin, &session, id, None).await
}

/// Renders the detail page, optionally with a form error from a
/// refused transition. A missing order is the styled admin 404.
async fn detail_response(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    id: i64,
    form_error: Option<String>,
) -> Response {
    let order = match order_service::find_order(pool, id).await {
        Ok(Some(order)) => order,
        Ok(None) => return record_not_found(pool, admin, session).await,
        Err(e) => return db_error("orders", e),
    };
    // The line items are this page's primary content: a failed
    // query is a 500, never an order that looks empty (the same
    // error-swallowing class as the 2026-11-02 totals fixes).
    let lines = match order_service::order_lines(pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("orders", e),
    };
    let legal = legal_next_statuses(&order.status);
    let can_cancel = legal.contains(&"cancelled");
    let next_statuses: Vec<NextStatus> = legal
        .iter()
        .filter(|s| **s != "cancelled")
        .map(|s| NextStatus {
            value: s,
            label: admin_status_label(s),
        })
        .collect();
    // The buyer name is decoration (the order and its lines above
    // are the page's substance): the service resolves it, logging
    // and degrading a failed lookup.
    let buyer = order_service::buyer_name(pool, &order).await;
    // The 変更履歴 reads the audit diary for this order. It is a
    // supporting section, not the page's substance: a failed read
    // logs and degrades to the empty state instead of failing the
    // whole page (the buyer-name rule).
    let history = match audit_query_service::order_history(pool, id).await {
        Ok(rows) => rows
            .iter()
            .map(|r| OrderHistoryRow {
                at: crate::models::jst::jst_datetime_label(r.created_at),
                admin_name: r.admin_name.clone(),
                action_label: action_label(&r.action).into_owned(),
            })
            .collect(),
        Err(e) => {
            tracing::error!("admin order history failed: {e}");
            Vec::new()
        }
    };
    // The payment block and its event history are supporting
    // sections like the buyer name: a failed read logs and
    // degrades (the page falls back to the COD wording), never
    // failing the whole detail page.
    let payment_row = match payment_repository::find_by_order_id(pool, id).await {
        Ok(row) => row,
        Err(e) => {
            tracing::error!(order_id = id, "admin order payment load failed: {e}");
            None
        }
    };
    let payment_label = payment_status_label_for_parts(
        &order.status,
        payment_row.as_ref().map(|p| p.provider.as_str()),
        payment_row.as_ref().map(|p| p.status.as_str()),
    );
    let payment_history: Vec<PaymentHistoryRow> = match &payment_row {
        Some(row) => match payment_repository::events_for_payment(pool, row.id).await {
            Ok(events) => events
                .iter()
                .map(|e| PaymentHistoryRow {
                    at: e.received_display(),
                    event_type: e.event_type.clone(),
                    applied_label: if e.applied {
                        "適用".to_string()
                    } else {
                        "記録のみ".to_string()
                    },
                })
                .collect(),
            Err(e) => {
                tracing::error!(order_id = id, "admin payment history failed: {e}");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let payment_block = payment_row
        .as_ref()
        .map(|row| admin_payment_block(row, &order.status));
    let tax_breakdown = match crate::repositories::order_repository::tax_breakdowns(pool, id).await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(order_id = id, "order tax breakdown failed: {e}");
            Vec::new()
        }
    };
    // Split shipments: the parcels and the still
    // unassigned remainder lines. Supporting sections like
    // the payment block: a failed read degrades to empty.
    let (shipments, split_lines) = build_shipment_sections(pool, id, &order.status).await;
    render(OrderDetailView {
        layout: layout_for(pool, admin, session).await,
        nav: "orders",
        placed: crate::models::jst::jst_long_datetime_label(order.created_at),
        status_label: admin_status_label(&order.status),
        buyer,
        member_id: order.user_id,
        can_cancel,
        has_actions: !next_statuses.is_empty() || can_cancel,
        next_statuses,
        lines,
        order,
        form_error,
        payment_status_label: payment_label,
        history,
        payment: payment_block,
        payment_history,
        tax_breakdown,
        disabled_carriers: crate::services::shop_settings_service::load(pool)
            .await
            .disabled_carriers,
        shipments,
        split_lines,
    })
}

/// Assembles the detail page's split-shipment sections
/// the parcels with their lines, and the remainder
/// lines the create form offers while the order is
/// `preparing`.
async fn build_shipment_sections(
    pool: &PgPool,
    order_id: i64,
    order_status: &str,
) -> (
    Vec<crate::views::admin::orders::AdminShipmentRow>,
    Vec<crate::views::admin::orders::SplitLineRow>,
) {
    use crate::repositories::admin::shipment_repository as ship_repo;
    let rows = ship_repo::list_for_order(pool, order_id)
        .await
        .unwrap_or_default();
    let mut shipments = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let items = ship_repo::items_for_shipment(pool, row.id)
            .await
            .unwrap_or_default();
        let items_label = items
            .iter()
            .map(|item| {
                if item.variant_label.is_empty() {
                    format!("{} ×{}", item.product_name, item.quantity)
                } else {
                    format!(
                        "{}（{}）×{}",
                        item.product_name, item.variant_label, item.quantity
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("、");
        shipments.push(crate::views::admin::orders::AdminShipmentRow {
            index: i + 1,
            id: row.id,
            items_label,
            carrier_label: crate::models::carrier::carrier_for(&row.carrier)
                .map(|c| c.label.to_string())
                .unwrap_or_default(),
            tracking_number: row.tracking_number.clone(),
            shipped: row.shipped_at.is_some(),
            mail_label: if row.mail_sent_at.is_some() {
                "送信済み".to_string()
            } else if row.shipped_at.is_some() {
                "送信待ち".to_string()
            } else {
                "未発送".to_string()
            },
        });
    }
    let mut split_lines = Vec::new();
    if order_status == "preparing" {
        let assigned: std::collections::BTreeMap<i64, i32> =
            ship_repo::assigned_quantities(pool, order_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .collect();
        if let Ok(named) = ship_repo::order_items_named(pool, order_id).await {
            for (item_id, name, variant, qty) in named {
                let remaining = qty - assigned.get(&item_id).copied().unwrap_or(0);
                if remaining > 0 {
                    split_lines.push(crate::views::admin::orders::SplitLineRow {
                        item_id,
                        label: if variant.is_empty() {
                            name
                        } else {
                            format!("{name}（{variant}）")
                        },
                        remaining,
                    });
                }
            }
        }
    }
    (shipments, split_lines)
}

/// Builds the detail's payment block from the payment row.
fn admin_payment_block(payment: &Payment, order_status: &str) -> AdminPaymentBlock {
    let method = payment.method_kind();
    AdminPaymentBlock {
        is_hosted: payment.is_hosted(),
        method_label: payment.method_label().to_string(),
        provider_label: payment_service::provider_display_name(&payment.provider).to_string(),
        status_label: payment_status_label_for_parts(
            order_status,
            Some(payment.provider.as_str()),
            Some(payment.status.as_str()),
        ),
        formatted_amount: payment.formatted_amount(),
        refunded: (payment.amount_refunded_yen > 0).then(|| payment.formatted_refunded()),
        refunded_yen: payment.amount_refunded_yen,
        session_id: payment.provider_session_id().map(str::to_string),
        payment_id: payment.provider_payment_id().map(str::to_string),
        deadline: payment.deadline_long(),
        captured_at: payment
            .captured_at
            .map(crate::models::jst::jst_datetime_label),
        flag_note: payment.flag_note.clone(),
        can_refund: payment.is_hosted()
            && payment.is_paid()
            && method != PaymentMethod::Konbini
            && payment.provider_payment_id().is_some(),
        can_sync: payment.is_hosted()
            && (payment.provider_payment_id().is_some() || payment.provider_session_id().is_some()),
        konbini_refund_note: payment.is_hosted()
            && method == PaymentMethod::Konbini
            && payment.is_paid(),
    }
}

/// Writes one payment audit row in its own transaction, after
/// the payment action itself has succeeded: the diary records
/// what an admin did, in the same all-or-nothing style as every
/// other audited action.
async fn record_payment_audit(
    pool: &PgPool,
    admin_id: i64,
    action: &'static str,
    order_id: i64,
) -> Result<(), crate::error::AppError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| crate::error::AppError::db("begin payment audit", e))?;
    crate::services::admin::audit_service::record(
        &mut tx,
        admin_id,
        action,
        "order",
        Some(order_id),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| crate::error::AppError::db("commit payment audit", e))?;
    Ok(())
}

/// The refund form body: an optional amount in yen (empty
/// refunds what remains).
#[derive(Deserialize)]
pub struct RefundForm {
    /// The refund amount as typed.
    #[serde(default)]
    amount_yen: String,
    /// The refunded total the page was rendered with: the
    /// staleness guard compares it against the live balance.
    #[serde(default)]
    expected_refunded_yen: String,
}

/// Handles a card/PayPay refund: the provider call runs through
/// the payment service (state changes only via its answer), and
/// the audit row follows success.
pub async fn refund(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<RefundForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let typed = form.amount_yen.trim();
    let amount = if typed.is_empty() {
        None
    } else {
        match typed.parse::<i64>() {
            Ok(value) => Some(value),
            Err(_) => {
                return detail_response(
                    &pool,
                    &admin,
                    &session,
                    id,
                    Some("返金額は円単位の数字で入力してください。".to_string()),
                )
                .await;
            }
        }
    };
    // The row's provider picks the refunding gateway: a
    // payment taken on either rail refunds through its own.
    let refund_provider = match payment_repository::find_by_order_id(&pool, id).await {
        Ok(Some(payment)) => payment.provider,
        Ok(None) => {
            return detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(PAYMENT_INFO_MISSING_MESSAGE.to_string()),
            )
            .await;
        }
        Err(e) => {
            return db_error(
                "orders",
                crate::error::AppError::db("refund payment load", e),
            );
        }
    };
    let Some(gateway) = payment_service::gateway_for(&refund_provider) else {
        return detail_response(
            &pool,
            &admin,
            &session,
            id,
            Some(format!(
                "{}が設定されていないため、返金できません。",
                payment_service::provider_display_name(&refund_provider)
            )),
        )
        .await;
    };
    // An unparsable expectation silently disables the
    // staleness guard inside the refund: the member path logs the equivalent loss, so
    // this path logs it too — the guard's absence must
    // never be invisible.
    let expected_refunded = form.expected_refunded_yen.trim().parse::<i64>().ok();
    if expected_refunded.is_none() {
        tracing::warn!(
            order_id = id,
            "admin refund: expected refunded total unparsable; staleness guard skipped"
        );
    }
    match payment_service::refund(&pool, &gateway, id, amount, expected_refunded).await {
        Ok(payment) => {
            if let Err(e) =
                record_payment_audit(&pool, admin.id, "payment.refund_requested", id).await
            {
                return db_error("orders", e);
            }
            set_flash(
                &session,
                &format!("返金を受け付けました（現在：{}）", payment.status_label()),
            )
            .await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Err(RefundError::NotEligible(message)) => {
            detail_response(&pool, &admin, &session, id, Some(message)).await
        }
        Err(RefundError::Gateway(e)) => {
            tracing::error!(order_id = id, "admin refund failed at the provider: {e}");
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(format!(
                    "{}で返金を受け付けられませんでした。時間をおいて再度お試しください。",
                    payment_service::provider_display_name(&refund_provider)
                )),
            )
            .await
        }
        Err(RefundError::Db(e)) => db_error("orders", e),
    }
}

/// Handles the manual sync: re-fetch this order's payment from
/// KOMOJU and apply what it says, for the moment a webhook is
/// late and the operator does not want to wait for the sweep.
pub async fn payment_sync(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let payment = match payment_repository::find_by_order_id(&pool, id).await {
        Ok(Some(payment)) => payment,
        Ok(None) => {
            return detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(PAYMENT_INFO_MISSING_MESSAGE.to_string()),
            )
            .await;
        }
        Err(e) => return db_error("orders", crate::error::AppError::db("sync payment load", e)),
    };
    // The row's provider picks the gateway to re-read from.
    let Some(gateway) = payment_service::gateway_for(&payment.provider) else {
        return detail_response(
            &pool,
            &admin,
            &session,
            id,
            Some(format!(
                "{}が設定されていないため、同期できません。",
                payment_service::provider_display_name(&payment.provider)
            )),
        )
        .await;
    };
    match payment_service::sync_from_provider(&pool, &gateway, &payment).await {
        Ok(_) => {
            if let Err(e) = record_payment_audit(&pool, admin.id, "payment.sync", id).await {
                return db_error("orders", e);
            }
            set_flash(&session, "支払い状況を同期しました").await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Err(e) => {
            tracing::error!(order_id = id, "admin payment sync failed: {e}");
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(format!(
                    "{}から支払い状況を取得できませんでした。",
                    payment_service::provider_display_name(&payment.provider)
                )),
            )
            .await
        }
    }
}

/// The status form body.
#[derive(Deserialize)]
pub struct StatusForm {
    /// Target status value.
    status: String,
    /// The status the form was rendered against, for the
    /// optimistic stale-state guard; absent in older forms, which
    /// skip the guard.
    #[serde(default)]
    expected: Option<String>,
    /// Carrier code posted with a ship transition; empty when
    /// the operator named none. Ignored for other targets.
    #[serde(default)]
    carrier: String,
    /// Tracking number posted with a ship transition; ignored
    /// for other targets.
    #[serde(default)]
    tracking_number: String,
}

/// The cancel form body: only the expected-status guard.
#[derive(Deserialize)]
pub struct CancelForm {
    /// See [`StatusForm::expected`].
    #[serde(default)]
    expected: Option<String>,
}

/// The stale-state message: the order moved after the page was
/// rendered.
const STALE_MESSAGE: &str = "ステータスが既に変更されています。最新の状態を確認してください。";

/// Handles the fulfilment status change.
pub async fn change_status(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<StatusForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match order_service::change_status(
        &pool,
        admin.id,
        id,
        &form.status,
        form.expected.as_deref(),
        &form.carrier,
        &form.tracking_number,
    )
    .await
    {
        Ok(()) => {
            let label = admin_status_label(&form.status);
            set_flash(&session, &format!("ステータスを「{label}」に更新しました")).await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Err(StatusChangeError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(StatusChangeError::Stale) => {
            set_flash(&session, STALE_MESSAGE).await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Err(StatusChangeError::IllegalTransition) => {
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(ORDER_STATUS_CHANGE_DENIED_MESSAGE.to_string()),
            )
            .await
        }
        Err(StatusChangeError::PaymentNotSettled) => {
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(SHIPMENT_BLOCKED_UNPAID_NOTICE.to_string()),
            )
            .await
        }
        Err(StatusChangeError::SplitShipment) => {
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(PARTIAL_SHIPMENT_UNIT_MESSAGE.to_string()),
            )
            .await
        }
        Err(StatusChangeError::InvalidShipping(message)) => {
            detail_response(&pool, &admin, &session, id, Some(message)).await
        }
        Err(StatusChangeError::Db(e)) => db_error("orders", e),
    }
}

/// Handles the cancellation (its own form, with the confirm).
pub async fn cancel(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<CancelForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match order_service::change_status(
        &pool,
        admin.id,
        id,
        "cancelled",
        form.expected.as_deref(),
        "",
        "",
    )
    .await
    {
        Ok(()) => {
            set_flash(&session, "注文をキャンセルしました").await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Err(StatusChangeError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(StatusChangeError::Stale) => {
            set_flash(&session, STALE_MESSAGE).await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Err(StatusChangeError::IllegalTransition) => {
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(ORDER_STATUS_CHANGE_DENIED_MESSAGE.to_string()),
            )
            .await
        }
        // Unreachable for cancellation (the guard watches the
        // shipped transition only); mapped for exhaustiveness.
        Err(StatusChangeError::PaymentNotSettled) => {
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(SHIPMENT_BLOCKED_UNPAID_NOTICE.to_string()),
            )
            .await
        }
        Err(StatusChangeError::SplitShipment) => {
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some(PARTIAL_SHIPMENT_UNIT_MESSAGE.to_string()),
            )
            .await
        }
        Err(StatusChangeError::InvalidShipping(message)) => {
            detail_response(&pool, &admin, &session, id, Some(message)).await
        }
        Err(StatusChangeError::Db(e)) => db_error("orders", e),
    }
}

/// Handles the shipping-mail resend: the recovery action for
/// a shipped order whose notification never landed (a failed
/// automatic send, or an order shipped before the feature
/// existed). Every outcome lands back on the order detail
/// with a flash that says exactly what happened.
pub async fn resend_shipping_mail(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    use crate::services::admin::order_service::{ResendMailError, ShippingMailOutcome};
    match order_service::resend_shipping_mail(&pool, admin.id, id).await {
        Ok(ShippingMailOutcome::Sent) => {
            set_flash(&session, "発送メールを再送信しました").await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Ok(ShippingMailOutcome::AlreadySent) => {
            set_flash(&session, "発送メールは送信済みです").await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Ok(ShippingMailOutcome::Failed) => {
            set_flash(
                &session,
                "発送メールの送信に失敗しました。時間をおいて再度お試しください。",
            )
            .await;
            Redirect::to(&format!("/admin/orders/{id}")).into_response()
        }
        Err(ResendMailError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(ResendMailError::NotShipped) => {
            detail_response(
                &pool,
                &admin,
                &session,
                id,
                Some("発送済みの注文でのみ再送信できます".to_string()),
            )
            .await
        }
        Err(ResendMailError::Db(e)) => db_error("orders", e),
    }
}

/// The bulk status targets: the forward flow's steps.
/// Cancellation is deliberately not a bulk target — it is a
/// per-order decision with its own confirmation and stock
/// compensation.
const BULK_TARGETS: [&str; 3] = ["preparing", "shipped", "delivered"];

/// Most orders one bulk POST may carry; a larger selection is
/// truncated with the cut announced on the confirmation page
/// (the products bulk rule).
const MAX_BULK_ORDERS: usize = 100;

/// The form body as raw (name, value) pairs: the bulk forms
/// repeat field names (one `order_id` per selected order),
/// which a struct-shaped form cannot express.
type FormPairs = Vec<(String, String)>;

/// The first value of one form field, or empty.
fn form_field(pairs: &FormPairs, name: &str) -> String {
    pairs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// Every value of one repeated form field, in posted order.
fn form_values(pairs: &FormPairs, name: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .collect()
}

/// The selected order IDs of a bulk form: parsed, deduplicated
/// in posted order, capped at [`MAX_BULK_ORDERS`]; the second
/// return value is how many a larger selection lost to the cap.
fn selected_order_ids(pairs: &FormPairs) -> (Vec<i64>, usize) {
    let mut ids: Vec<i64> = Vec::new();
    for raw in form_values(pairs, "order_id") {
        if let Ok(id) = raw.trim().parse::<i64>()
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    let truncated = ids.len().saturating_sub(MAX_BULK_ORDERS);
    ids.truncate(MAX_BULK_ORDERS);
    (ids, truncated)
}

/// The bulk confirmation page: the operator sees exactly
/// which orders the target can reach before anything is
/// written (the back-office confirmation rule). Nothing is
/// applied here.
pub async fn bulk_preview(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<FormPairs>,
) -> Response {
    let return_url = super::safe_return_url(&pairs, "/admin/orders");
    let target = form_field(&pairs, "target");
    if !BULK_TARGETS.contains(&target.as_str()) {
        set_flash(&session, BULK_STATUS_INVALID_MESSAGE).await;
        return Redirect::to(&return_url).into_response();
    }
    let (ids, truncated) = selected_order_ids(&pairs);
    if ids.is_empty() {
        set_flash(&session, NO_ORDERS_SELECTED_MESSAGE).await;
        return Redirect::to(&return_url).into_response();
    }
    let mut rows = Vec::with_capacity(ids.len());
    let mut applicable_count = 0usize;
    for id in &ids {
        let order = match order_service::find_order(&pool, *id).await {
            Ok(Some(order)) => order,
            Ok(None) => continue,
            Err(e) => return db_error("orders", e),
        };
        let applicable = legal_next_statuses(&order.status).contains(&target.as_str());
        if applicable {
            applicable_count += 1;
        }
        rows.push(BulkConfirmRow {
            id: order.id,
            number: order.order_number(),
            buyer: order_service::buyer_name(&pool, &order).await,
            current_label: admin_status_label(&order.status),
            expected: order.status.clone(),
            applicable,
        });
    }
    render(BulkConfirmView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "orders",
        rows,
        applicable_count,
        target_label: admin_status_label(&target),
        is_shipped: target == "shipped",
        carriers: {
            let settings = crate::services::shop_settings_service::load(&pool).await;
            crate::models::carrier::CARRIERS
                .iter()
                .filter(|c| !settings.carrier_disabled(c.code))
                .map(|c| BulkCarrierOption {
                    code: c.code,
                    label: c.label,
                })
                .collect()
        },
        target,
        return_url,
        truncated,
    })
}

/// Applies the confirmed bulk status change: one transaction
/// per order inside the service, skipped orders counted by
/// reason in the flash, then back to the list the operator
/// was working in (filters preserved by the return URL).
pub async fn bulk_execute(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<FormPairs>,
) -> Response {
    let return_url = super::safe_return_url(&pairs, "/admin/orders");
    let target = form_field(&pairs, "target");
    if !BULK_TARGETS.contains(&target.as_str()) {
        set_flash(&session, BULK_STATUS_INVALID_MESSAGE).await;
        return Redirect::to(&return_url).into_response();
    }
    let (ids, _) = selected_order_ids(&pairs);
    let expected = form_values(&pairs, "expected_status");
    // The expected statuses pair with the IDs by position; a
    // short or mismatched list falls back to an empty guard
    // value, which the service reads as a stale state — a
    // malformed form applies nothing rather than guessing.
    let items: Vec<(i64, String)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, expected.get(i).cloned().unwrap_or_default()))
        .collect();
    if items.is_empty() {
        set_flash(&session, NO_ORDERS_SELECTED_MESSAGE).await;
        return Redirect::to(&return_url).into_response();
    }
    let carrier = form_field(&pairs, "carrier");
    let outcomes =
        match order_service::change_status_bulk(&pool, admin.id, &items, &target, &carrier).await {
            Ok(v) => v,
            Err(StatusChangeError::InvalidShipping(message)) => {
                set_flash(&session, &message).await;
                return Redirect::to(&return_url).into_response();
            }
            Err(StatusChangeError::Db(e)) => return db_error("orders", e),
            Err(_) => {
                // The bulk service folds every per-order refusal
                // into its outcomes; only the two arms above can
                // reach here in practice.
                tracing::error!("bulk status change returned an unexpected error");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
                    .into_response();
            }
        };
    let applied = outcomes.iter().filter(|o| o.applied).count();
    let label = admin_status_label(&target);
    let mut message = format!("{applied}件を「{label}」に更新しました");
    // Skipped orders grouped by reason: the operator learns
    // what did not move and why, in the one flash line.
    let mut reasons: Vec<(String, usize)> = Vec::new();
    for outcome in outcomes.iter().filter(|o| !o.applied) {
        match reasons.iter_mut().find(|(r, _)| *r == outcome.reason) {
            Some((_, count)) => *count += 1,
            None => reasons.push((outcome.reason.clone(), 1)),
        }
    }
    if !reasons.is_empty() {
        let parts: Vec<String> = reasons
            .iter()
            .map(|(reason, count)| format!("{reason}：{count}件"))
            .collect();
        message.push_str(&format!("（対象外 — {}）", parts.join("、")));
    }
    set_flash(&session, &message).await;
    Redirect::to(&return_url).into_response()
}

/// Body limit for the tracking CSV upload (the product and
/// category imports' shared size).
const CSV_BODY_LIMIT: usize = 12 * 1024 * 1024;

/// Downloads one carrier's label CSV for the selected orders
/// (or every shippable unshipped order when nothing is
/// selected). A download cannot carry a flash message, so
/// when nothing at all can ship — most often because every
/// candidate is waiting on payment — the operator gets the
/// explanation on the list instead of an empty file.
pub async fn label_csv(
    State(pool): State<PgPool>,
    CurrentAdmin(_admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<FormPairs>,
) -> Response {
    let return_url = super::safe_return_url(&pairs, "/admin/orders");
    let carrier = form_field(&pairs, "label_carrier");
    let (ids, _) = selected_order_ids(&pairs);
    let ids_arg = (!ids.is_empty()).then_some(ids.as_slice());
    let settings = crate::services::shop_settings_service::load(&pool).await;
    let today = crate::models::jst::jst_today();
    match crate::services::admin::shipping_csv_service::export_labels(
        &pool,
        &carrier,
        ids_arg,
        &settings.shop_name,
        today,
    )
    .await
    {
        Ok(export) if export.order_count > 0 => {
            let disposition = format!("attachment; filename=\"{}\"", export.filename);
            (
                [
                    (header::CONTENT_TYPE, "text/csv; charset=Shift_JIS"),
                    (header::CONTENT_DISPOSITION, disposition.as_str()),
                ],
                export.bytes,
            )
                .into_response()
        }
        Ok(export) => {
            let message = if export.skipped_unsettled > 0 {
                format!(
                    "出力できる注文がありません（未発送 {} 件は入金待ちのため発送できません）",
                    export.skipped_unsettled
                )
            } else {
                "出力できる未発送の注文がありません".to_string()
            };
            set_flash(&session, &message).await;
            Redirect::to(&return_url).into_response()
        }
        Err(crate::error::AppError::NotFound) => {
            set_flash(&session, CARRIER_SELECTION_INVALID_MESSAGE).await;
            Redirect::to(&return_url).into_response()
        }
        Err(e) => db_error("orders", e),
    }
}

/// Receives the tracking-number CSV (multipart, field
/// `file`): applies it row by row through the shipping CSV
/// service and renders the per-row results page — the one
/// admin POST that answers with a page instead of a redirect,
/// because the results ARE the page (the import report
/// pattern). A file whose header does not match the format
/// never starts: the operator is sent back with the reason.
pub async fn tracking_import(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    mut multipart: Multipart,
) -> Response {
    let mut bytes: Option<Vec<u8>> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        match field.bytes().await {
            Ok(b) => bytes = Some(b.to_vec()),
            Err(_) => break,
        }
    }
    let Some(bytes) = bytes.filter(|b| !b.is_empty()) else {
        set_flash(&session, CSV_FILE_REQUIRED_MESSAGE).await;
        return Redirect::to("/admin/orders").into_response();
    };
    if bytes.len() as i64 > CONFIG.csv_max_file_bytes {
        set_flash(&session, CSV_FILE_TOO_LARGE_MESSAGE).await;
        return Redirect::to("/admin/orders").into_response();
    }
    let text = crate::util::csv_transfer::decode_bytes(&bytes);
    match crate::services::admin::shipping_csv_service::import_tracking(&pool, admin.id, &text)
        .await
    {
        Ok(report) => {
            let applied_count = report.applied_count();
            render(TrackingImportView {
                layout: layout_for(&pool, &admin, &session).await,
                nav: "orders",
                rows: report
                    .rows
                    .into_iter()
                    .map(|r| TrackingImportRow {
                        line: r.line,
                        number: r.number,
                        applied: r.applied,
                        message: r.message,
                    })
                    .collect(),
                applied_count,
            })
        }
        Err(line_error) => {
            set_flash(
                &session,
                &format!("CSVを取り込めませんでした: {}", line_error.message),
            )
            .await;
            Redirect::to("/admin/orders").into_response()
        }
    }
}

/// The bulk packing-slip print page for the selected orders
/// （納品書）: a standalone print document, one slip per order,
/// sharing the receipt page's data (snapshot lines, stored
/// tax breakdown, settings shop name). Reached by GET so the
/// operator can bookmark or re-print the exact selection.
pub async fn packing_slips(
    State(pool): State<PgPool>,
    CurrentAdmin(_admin): CurrentAdmin,
    session: Session,
    Query(pairs): Query<FormPairs>,
) -> Response {
    let (ids, _) = selected_order_ids(&pairs);
    if ids.is_empty() {
        set_flash(&session, NO_ORDERS_SELECTED_MESSAGE).await;
        return Redirect::to("/admin/orders").into_response();
    }
    let settings = crate::services::shop_settings_service::load(&pool).await;
    let today = crate::models::jst::jst_today();
    let mut slips = Vec::with_capacity(ids.len());
    for id in &ids {
        let order = match order_service::find_order(&pool, *id).await {
            Ok(Some(order)) => order,
            Ok(None) => continue,
            Err(e) => return db_error("orders", e),
        };
        let lines = match order_service::order_lines(&pool, *id).await {
            Ok(v) => v,
            Err(e) => return db_error("orders", e),
        };
        let tax_breakdown =
            match crate::repositories::order_repository::tax_breakdowns(&pool, *id).await {
                Ok(v) => v,
                Err(e) => {
                    return db_error(
                        "orders",
                        crate::error::AppError::db("packing slip tax breakdown", e),
                    );
                }
            };
        slips.push(PackingSlip {
            order,
            lines,
            tax_breakdown,
        });
    }
    if slips.is_empty() {
        set_flash(&session, "注文が見つかりませんでした。").await;
        return Redirect::to("/admin/orders").into_response();
    }
    render(PackingSlipsView {
        asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
        shop_name: settings.shop_name.clone(),
        registration_number: settings.invoice_registration_number.clone(),
        issued_label: crate::models::delivery::format_delivery_date_long(today),
        slips,
    })
}

/// Parses one `YYYY-MM-DD` date input into the UTC instant the
/// JST calendar day starts; an empty or malformed value means
/// the date filter stays open on that side.
fn jst_day_start(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let date = chrono::NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").ok()?;
    date.and_hms_opt(0, 0, 0)?
        .and_local_timezone(crate::models::jst::jst())
        .single()
        .map(|at| at.with_timezone(&chrono::Utc))
}

/// Creates one open split shipment from the detail
/// page's per-line quantity inputs (`ship_qty_{item_id}`).
/// Both shipment handlers answer with the detail page
/// re-rendered (the back-office same-screen rule), errors
/// surfacing as the page's form error.
async fn create_shipment(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<i64>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let items: Vec<(i64, i32)> = pairs
        .iter()
        .filter_map(|(name, value)| {
            let item_id = name.strip_prefix("ship_qty_")?.parse::<i64>().ok()?;
            let qty = value.trim().parse::<i32>().ok()?;
            Some((item_id, qty))
        })
        .collect();
    match crate::services::admin::shipment_service::create_shipment(&pool, admin.id, id, &items)
        .await
    {
        Ok(_) => {
            set_flash(&session, "出荷を作成しました。").await;
            detail_response(&pool, &admin, &session, id, None).await
        }
        Err(e) => {
            let message = match &e {
                crate::error::AppError::Conflict(m) => m.clone(),
                _ => ADMIN_PROCESSING_FAILED_MESSAGE.to_string(),
            };
            detail_response(&pool, &admin, &session, id, Some(message)).await
        }
    }
}

/// Ships one open split shipment with the posted
/// carrier and tracking number; its notification queues
/// inside the ship transaction.
async fn ship_shipment(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path((id, sid)): Path<(i64, i64)>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let field = |name: &str| {
        pairs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    };
    match crate::services::admin::shipment_service::ship_shipment(
        &pool,
        admin.id,
        id,
        sid,
        &field("carrier"),
        &field("tracking_number"),
    )
    .await
    {
        Ok(completed) => {
            set_flash(
                &session,
                if completed {
                    "分割出荷を発送し、注文は発送済みになりました。"
                } else {
                    "分割出荷を発送しました。"
                },
            )
            .await;
            detail_response(&pool, &admin, &session, id, None).await
        }
        Err(e) => {
            let message = match &e {
                crate::error::AppError::Conflict(m) => m.clone(),
                _ => ADMIN_PROCESSING_FAILED_MESSAGE.to_string(),
            };
            detail_response(&pool, &admin, &session, id, Some(message)).await
        }
    }
}
