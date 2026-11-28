//! Split-shipment operations: creating parcels of an
//! order and shipping them one by one. The order's
//! fulfilment state derives from its parcels — the order
//! turns `shipped` in the same transaction that ships the
//! last open quantity. Orders without parcels never touch
//! this service: the classic single-shipment path on the
//! order row stays the default.
//!
//! A parcel's notification goes through the shipping-mail
//! outbox like every bulk shipment: one queued row per
//! parcel, stamped on the parcel itself. The mail reuses
//! the shop's shipping template with the parcel's carrier,
//! tracking number and lines; the money blocks (totals,
//! tax) stay order-level because the invoice is per order
//! in this shop.

use std::collections::BTreeMap;

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    CARRIER_INVALID_MESSAGE, ORDER_NOT_FOUND_MESSAGE, SHIPMENT_BLOCKED_UNPAID_MESSAGE,
};
use crate::repositories::admin::shipment_repository as repo;
use crate::repositories::{order_repository, payment_repository};
use crate::services::admin::audit_service;
use crate::services::admin::order_service::ShippingMailOutcome;

/// Creates an open parcel holding the given per-line
/// quantities. Only a `preparing` order can be split: the
/// operator moves a fresh order there first, exactly like
/// the classic flow. Quantities beyond what remains
/// unassigned are refused — a parcel can never promise
/// more than the order holds.
pub async fn create_shipment(
    pool: &PgPool,
    admin_id: i64,
    order_id: i64,
    items: &[(i64, i32)],
) -> Result<i64, AppError> {
    let chosen: Vec<(i64, i32)> = items.iter().copied().filter(|(_, qty)| *qty > 0).collect();
    if chosen.is_empty() {
        return Err(AppError::Conflict(
            "出荷する商品の数量を指定してください".to_string(),
        ));
    }
    let mut tx = pool.begin().await?;
    let order = order_repository::lock_by_id(&mut tx, order_id)
        .await?
        .ok_or_else(|| AppError::Conflict(ORDER_NOT_FOUND_MESSAGE.to_string()))?;
    if order.status != "preparing" {
        return Err(AppError::Conflict(
            "分割できるのは「出荷準備中」の注文だけです。先に状態を進めてください".to_string(),
        ));
    }
    let ordered: BTreeMap<i64, i32> = repo::order_items_in_tx(&mut tx, order_id)
        .await?
        .into_iter()
        .collect();
    let assigned: BTreeMap<i64, i32> = repo::assigned_quantities_in_tx(&mut tx, order_id)
        .await?
        .into_iter()
        .collect();
    for (item_id, qty) in &chosen {
        let Some(total) = ordered.get(item_id) else {
            return Err(AppError::Conflict(
                "注文に含まれない商品が指定されました".to_string(),
            ));
        };
        let used = assigned.get(item_id).copied().unwrap_or(0);
        if used + qty > *total {
            return Err(AppError::Conflict(
                "未出荷の数量を超える指定があります".to_string(),
            ));
        }
    }
    let shipment_id = repo::insert_shipment(&mut tx, order_id).await?;
    for (item_id, qty) in &chosen {
        repo::insert_item(&mut tx, shipment_id, *item_id, *qty).await?;
    }
    audit_service::record(
        &mut tx,
        admin_id,
        "order.shipment_create",
        "order",
        Some(order_id),
    )
    .await?;
    tx.commit().await?;
    Ok(shipment_id)
}

/// Ships one open parcel: validates the carrier, applies
/// the same unsettled-payment guard as the single-order
/// ship (TC-17), queues the parcel's notification in the
/// same transaction, and moves the order itself to
/// `shipped` when this parcel carries the last open
/// quantity. Returns whether the order completed.
pub async fn ship_shipment(
    pool: &PgPool,
    admin_id: i64,
    order_id: i64,
    shipment_id: i64,
    carrier_code: &str,
    tracking_number: &str,
) -> Result<bool, AppError> {
    if crate::models::carrier::carrier_for(carrier_code).is_none() {
        return Err(AppError::Conflict(CARRIER_INVALID_MESSAGE.to_string()));
    }
    let mut tx = pool.begin().await?;
    let order = order_repository::lock_by_id(&mut tx, order_id)
        .await?
        .ok_or_else(|| AppError::Conflict(ORDER_NOT_FOUND_MESSAGE.to_string()))?;
    if !matches!(order.status.as_str(), "preparing" | "shipped") {
        return Err(AppError::Conflict(
            "この注文は出荷できる状態ではありません".to_string(),
        ));
    }
    // TC-17, identical to the single-order ship: hosted
    // money that has not settled never leaves the shelf.
    let payment = payment_repository::find_by_order_id(&mut *tx, order_id).await?;
    if payment
        .as_ref()
        .is_some_and(|p| p.is_hosted() && !p.is_paid())
    {
        return Err(AppError::Conflict(
            SHIPMENT_BLOCKED_UNPAID_MESSAGE.to_string(),
        ));
    }
    let shipment = repo::find_in_tx(&mut tx, shipment_id)
        .await?
        .filter(|s| s.order_id == order_id)
        .ok_or_else(|| AppError::Conflict("出荷が見つかりません".to_string()))?;
    if shipment.shipped_at.is_some() {
        return Err(AppError::Conflict(
            "この出荷はすでに発送済みです".to_string(),
        ));
    }
    repo::mark_shipped(&mut tx, shipment_id, carrier_code, tracking_number).await?;
    crate::repositories::shipping_mail_repository::enqueue_shipment_tx(
        &mut tx,
        order_id,
        shipment_id,
    )
    .await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "order.shipment_ship",
        "order",
        Some(order_id),
    )
    .await?;
    // The derivation: every ordered quantity assigned AND
    // every parcel shipped means the order has shipped.
    let ordered: BTreeMap<i64, i32> = repo::order_items_in_tx(&mut tx, order_id)
        .await?
        .into_iter()
        .collect();
    let assigned: BTreeMap<i64, i32> = repo::assigned_quantities_in_tx(&mut tx, order_id)
        .await?
        .into_iter()
        .collect();
    let all_assigned = ordered
        .iter()
        .all(|(item, qty)| assigned.get(item).copied().unwrap_or(0) >= *qty);
    let open_left = repo::open_count_in_tx(&mut tx, order_id).await?;
    let completed = all_assigned && open_left == 0 && order.status == "preparing";
    if completed {
        crate::repositories::admin::order_repository::set_order_status(
            &mut tx, order_id, "shipped",
        )
        .await?;
        // Points earn grants when the order completes its
        // shipment (EC V2 core 1), the same hook the
        // whole-order ship calls.
        let points = crate::services::shop_settings_service::load(pool)
            .await
            .points_settings();
        crate::services::point_service::grant_earn_for_order(&mut tx, &order, points).await?;
        audit_service::record(&mut tx, admin_id, "order.status", "order", Some(order_id)).await?;
    }
    tx.commit().await?;
    Ok(completed)
}

/// Sends one parcel's shipping notification and stamps the
/// parcel on success. Mirrors the order-level send's claim
/// discipline: the conditional stamp decides which of two
/// racing sends counts, and the lock is never held across
/// the SMTP call.
pub async fn send_shipment_mail(
    pool: &PgPool,
    order_id: i64,
    shipment_id: i64,
    shipped_date: Option<chrono::NaiveDate>,
) -> ShippingMailOutcome {
    let shipment = match repo::find(pool, shipment_id).await {
        Ok(Some(row)) => row,
        _ => return ShippingMailOutcome::Failed,
    };
    if shipment.mail_sent_at.is_some() {
        return ShippingMailOutcome::AlreadySent;
    }
    let mut order = match order_repository::find_by_id(pool, order_id).await {
        Ok(Some(order)) => order,
        _ => return ShippingMailOutcome::Failed,
    };
    // The parcel's own carrier/tracking ride the shared
    // template through the order shape it already renders.
    order.carrier = Some(shipment.carrier.clone());
    order.tracking_number = Some(shipment.tracking_number.clone());
    let lines = match repo::mail_lines_for_shipment(pool, shipment_id).await {
        Ok(lines) => lines,
        Err(_) => return ShippingMailOutcome::Failed,
    };
    let breakdown = order_repository::tax_breakdowns(pool, order_id)
        .await
        .unwrap_or_default();
    let sent =
        crate::mail::send_shipping_notification(pool, &order, &lines, &breakdown, shipped_date)
            .await
            .is_ok();
    if !sent {
        tracing::warn!(
            order_id,
            shipment_id,
            action = "order.shipment_mail",
            "shipment mail send failed"
        );
        return ShippingMailOutcome::Failed;
    }
    match repo::stamp_mail_sent(pool, shipment_id).await {
        Ok(true) => ShippingMailOutcome::Sent,
        Ok(false) => ShippingMailOutcome::AlreadySent,
        Err(_) => ShippingMailOutcome::Failed,
    }
}
