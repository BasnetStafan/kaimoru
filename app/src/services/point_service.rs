//! Points programme service (EC V2 core 1;
//! internal points design record).
//!
//! The rules, in one place: points are a payment
//! instrument (they never change an order's total or its
//! tax), earn is granted at shipment on the amount the
//! member actually paid for goods, spend is FIFO across
//! grant lots inside the placement transaction, and every
//! order reversal walks the ledger backwards exactly once.

use chrono::{Duration, Utc};
use sqlx::{PgConnection, PgPool};
use tokio::sync::watch::Receiver;

use crate::config::CONFIG;

use crate::error::AppError;
use crate::models::points::{
    GrantLot, KIND_EARN, KIND_EARN_REVOKE, KIND_EXPIRE, KIND_SPEND, KIND_SPEND_REFUND,
};
use crate::repositories::point_repository;

/// The member's spendable balance (unexpired grant
/// remainders).
pub async fn balance(pool: &PgPool, user_id: i64) -> Result<i64, AppError> {
    Ok(point_repository::balance(pool, user_id).await?)
}

/// The points an order earns: `earn_percent` of the earn
/// base (merchandise after the coupon discount and the
/// points spent), floored. Pure, so the rule is testable
/// without a database.
pub fn earned_points(earn_base_yen: i64, earn_percent: i32) -> i64 {
    if earn_base_yen <= 0 || earn_percent <= 0 {
        return 0;
    }
    earn_base_yen * i64::from(earn_percent) / 100
}

/// The points an order earns when its lines carry
/// different rates (the override layer): the order-level
/// deduction (coupon discount + points spent) is allocated
/// across the lines pro-rata by line amount — floored per
/// line, the rounding residual to the largest line — and
/// each line earns at its own snapshotted rate on its
/// share of the base. A deduction at or above the
/// subtotal earns nothing. Pure.
pub fn earned_points_for_lines(lines: &[(i64, i32)], deduction_yen: i64) -> i64 {
    let subtotal: i64 = lines.iter().map(|(amount, _)| amount).sum();
    if subtotal <= 0 || deduction_yen >= subtotal {
        return 0;
    }
    let mut shares = vec![0i64; lines.len()];
    if deduction_yen > 0 {
        let mut allocated = 0i64;
        for (i, (amount, _)) in lines.iter().enumerate() {
            shares[i] = deduction_yen * amount / subtotal;
            allocated += shares[i];
        }
        let residual = deduction_yen - allocated;
        if residual > 0
            && let Some((largest, _)) = lines
                .iter()
                .enumerate()
                .max_by_key(|(_, (amount, _))| *amount)
        {
            shares[largest] += residual;
        }
    }
    lines
        .iter()
        .zip(&shares)
        .map(|((amount, rate), share)| earned_points(amount - share, *rate))
        .sum()
}

/// Plans a spend across grant lots in the order given
/// (the repository returns them expiry-first): each lot
/// contributes up to its remainder. Returns the
/// (grant id, points) takes, or `None` when the lots
/// cannot cover the spend. Pure.
pub fn plan_spend(lots: &[GrantLot], want: i64) -> Option<Vec<(i64, i64)>> {
    if want < 0 {
        return None;
    }
    let mut left = want;
    let mut takes = Vec::new();
    for lot in lots {
        if left == 0 {
            break;
        }
        let take = lot.remaining.min(left);
        if take > 0 {
            takes.push((lot.id, take));
            left -= take;
        }
    }
    if left == 0 { Some(takes) } else { None }
}

/// Spends `points` of the member's balance for an order,
/// inside the caller's transaction (order placement).
/// The grants are locked first, so the balance the
/// confirm screen showed is re-proven here: if it moved,
/// the spend refuses with a conflict and placement
/// re-renders the checkout (the price-drift pattern).
pub async fn spend_in_tx(
    conn: &mut PgConnection,
    user_id: i64,
    order_id: i64,
    points: i64,
) -> Result<(), AppError> {
    if points <= 0 {
        return Ok(());
    }
    let lots = point_repository::lock_spendable_grants(conn, user_id).await?;
    let takes = plan_spend(&lots, points).ok_or_else(|| {
        AppError::Conflict(
            "ポイント残高が変わりました。お手数ですが、ご注文内容をご確認ください。".to_string(),
        )
    })?;
    let spend_id = point_repository::insert_movement(
        conn,
        user_id,
        KIND_SPEND,
        -points,
        None,
        Some(order_id),
        None,
    )
    .await?;
    for (grant_id, take) in takes {
        point_repository::reduce_grant_remaining(conn, grant_id, take).await?;
        point_repository::insert_allocation(conn, spend_id, grant_id, take).await?;
    }
    Ok(())
}

/// Grants an order's earn, inside the caller's
/// transaction (the shipment transition). Idempotent: an
/// order earns at most once. Returns the points granted
/// (0 when the earn computes to nothing).
pub async fn earn_in_tx(
    conn: &mut PgConnection,
    user_id: i64,
    order_id: i64,
    earn_base_yen: i64,
    earn_percent: i32,
    expiry_days: i32,
) -> Result<i64, AppError> {
    if point_repository::earn_for_order(conn, order_id)
        .await?
        .is_some()
    {
        return Ok(0);
    }
    let points = earned_points(earn_base_yen, earn_percent);
    grant_points_in_tx(conn, user_id, order_id, points, expiry_days).await
}

/// Grants a precomputed earn amount, inside the caller's
/// transaction: the shared tail of the order-level and
/// per-line computations. Idempotent — an order earns at
/// most once — and a non-positive amount writes nothing.
/// Returns the points granted.
pub async fn grant_points_in_tx(
    conn: &mut PgConnection,
    user_id: i64,
    order_id: i64,
    points: i64,
    expiry_days: i32,
) -> Result<i64, AppError> {
    if point_repository::earn_for_order(conn, order_id)
        .await?
        .is_some()
    {
        return Ok(0);
    }
    if points <= 0 {
        return Ok(0);
    }
    let expires_at = Utc::now() + Duration::days(i64::from(expiry_days.max(1)));
    point_repository::insert_movement(
        conn,
        user_id,
        KIND_EARN,
        points,
        Some(points),
        Some(order_id),
        Some(expires_at),
    )
    .await?;
    Ok(points)
}

/// Grants an order's earn at its shipment, inside the
/// caller's transaction: the shared hook both ship paths
/// (whole-order and split-shipment completion) call, so
/// an order earns exactly once however it ships. Guests
/// and a disabled programme earn nothing. The granted
/// figure is stamped on the order in the same transaction.
pub async fn grant_earn_for_order(
    conn: &mut PgConnection,
    order: &crate::models::Order,
    settings: crate::models::shop_settings::PointsSettings,
) -> Result<(), AppError> {
    if !settings.enabled {
        return Ok(());
    }
    let Some(user_id) = order.user_id else {
        return Ok(());
    };
    // Override layer: lines snapshotted at placement earn
    // per line at their own rates; an order whose lines
    // predate the snapshot (any rate missing) keeps the
    // original order-level computation.
    let facts =
        crate::repositories::order_repository::line_point_facts(&mut *conn, order.id).await?;
    let granted = if !facts.is_empty() && facts.iter().all(|(_, rate)| rate.is_some()) {
        let lines: Vec<(i64, i32)> = facts
            .iter()
            .map(|(amount, rate)| (*amount, rate.unwrap_or(0)))
            .collect();
        let points = earned_points_for_lines(&lines, order.discount_yen + order.points_used_yen);
        grant_points_in_tx(conn, user_id, order.id, points, settings.expiry_days).await?
    } else {
        let earn_base = (order.subtotal_yen - order.discount_yen - order.points_used_yen).max(0);
        earn_in_tx(
            conn,
            user_id,
            order.id,
            earn_base,
            settings.earn_percent,
            settings.expiry_days,
        )
        .await?
    };
    if granted > 0 {
        crate::repositories::order_repository::set_points_earned(&mut *conn, order.id, granted)
            .await?;
    }
    Ok(())
}

/// Reverses every points movement of an order, inside the
/// caller's transaction (order cancellation): spends are
/// refunded to the exact grant lots they consumed (an
/// expired lot hands its restored points a fresh validity
/// instead — see `restore_allocation`), and a granted earn
/// is revoked down to its unspent remainder (points
/// already re-spent are not clawed back — the v1 rule in
/// the spec). Idempotent: a second call finds the
/// allocations reversed and the revocation recorded, and
/// writes nothing. `expiry_days` is the shop's configured
/// points expiry window, read by the caller before its
/// transaction opened (the A6 settings pattern).
pub async fn reverse_order_in_tx(
    conn: &mut PgConnection,
    user_id: i64,
    order_id: i64,
    expiry_days: i32,
) -> Result<(), AppError> {
    for spend in point_repository::spends_for_order(conn, order_id).await? {
        let allocations = point_repository::allocations_for_spend(conn, spend.id).await?;
        if allocations.is_empty() {
            continue;
        }
        let mut restored = 0i64;
        for (grant_id, points) in &allocations {
            restore_allocation(conn, *grant_id, *points, expiry_days).await?;
            restored += points;
        }
        point_repository::mark_spend_reversed(conn, spend.id).await?;
        point_repository::insert_movement(
            conn,
            user_id,
            KIND_SPEND_REFUND,
            restored,
            None,
            Some(order_id),
            None,
        )
        .await?;
    }
    if !point_repository::movement_exists_for_order(conn, order_id, KIND_EARN_REVOKE).await?
        && let Some(grant) = point_repository::earn_for_order(conn, order_id).await?
    {
        let remaining = grant.remaining.unwrap_or(0);
        if remaining > 0 {
            point_repository::reduce_grant_remaining(conn, grant.id, remaining).await?;
            point_repository::insert_movement(
                conn,
                user_id,
                KIND_EARN_REVOKE,
                -remaining,
                None,
                Some(order_id),
                None,
            )
            .await?;
        }
    }
    Ok(())
}

/// Restores one spend allocation to its grant lot,
/// inside the caller's transaction. A lot still inside
/// its validity takes the points back as-is. A lot whose
/// expiry has already passed follows the owner decision
/// of 2026-11-22 (the PayPay/Ponta pattern, recorded in
/// internal points design record): the lot's unspent remainder is
/// first expired exactly as the sweep would expire it —
/// never revived — and the restored points alone come
/// back, with a fresh validity of `expiry_days` counted
/// from now. The lot row is locked before either read,
/// so the expiry sweep cannot interleave: if the sweep
/// locked first, the remainder already reads zero and
/// only the restore lands; if this reversal locked
/// first, the sweep's re-check sees the new future
/// expiry and skips the lot.
async fn restore_allocation(
    conn: &mut PgConnection,
    grant_id: i64,
    points: i64,
    expiry_days: i32,
) -> Result<(), AppError> {
    let Some(grant) = point_repository::lock_grant(conn, grant_id).await? else {
        // The grant row is gone: the bare restore updated
        // nothing in that case too, and failing the whole
        // reversal over an unreconstructable lot would
        // strand the refund it rides with.
        return Ok(());
    };
    let expired = grant
        .expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now());
    if !expired {
        point_repository::restore_grant_remaining(conn, grant_id, points).await?;
        return Ok(());
    }
    let stale = grant.remaining.unwrap_or(0);
    if stale > 0 {
        point_repository::reduce_grant_remaining(conn, grant_id, stale).await?;
        point_repository::insert_movement(
            conn,
            grant.user_id,
            KIND_EXPIRE,
            -stale,
            None,
            grant.order_id,
            None,
        )
        .await?;
    }
    point_repository::restore_grant_remaining(conn, grant_id, points).await?;
    let fresh_expiry = Utc::now() + Duration::days(i64::from(expiry_days.max(1)));
    point_repository::set_grant_expiry(conn, grant_id, fresh_expiry).await?;
    Ok(())
}

/// Sweeps grants whose expiry passed with points unspent:
/// each becomes an `expire` movement and a zero remainder.
/// Runs on the mail-worker interval; returns how many
/// grants expired.
///
/// Per-grant transactions: the
/// batch is SELECTED first, then each grant expires in
/// its own transaction, so one failing grant rolls back
/// only itself — never the other 499 — and the pass logs
/// it and moves on instead of wedging on the same oldest
/// grants every interval.
pub async fn expire_due(pool: &PgPool) -> Result<i64, AppError> {
    let candidates = point_repository::expired_grant_candidates(pool, 500).await?;
    let mut expired = 0i64;
    for candidate in &candidates {
        match expire_one(pool, candidate.id).await {
            Ok(true) => expired += 1,
            Ok(false) => {}
            Err(e) => {
                tracing::error!(
                    grant_id = candidate.id,
                    "points expiry failed for one grant; continuing: {e}"
                );
            }
        }
    }
    Ok(expired)
}

/// Expires ONE grant in its own transaction; `false`
/// when the grant stopped qualifying after selection.
async fn expire_one(pool: &PgPool, grant_id: i64) -> Result<bool, AppError> {
    let mut tx = pool.begin().await?;
    let Some(grant) = point_repository::lock_expired_grant(&mut tx, grant_id).await? else {
        return Ok(false);
    };
    let remaining = grant.remaining.unwrap_or(0);
    if remaining <= 0 {
        return Ok(false);
    }
    point_repository::reduce_grant_remaining(&mut tx, grant.id, remaining).await?;
    point_repository::insert_movement(
        &mut tx,
        grant.user_id,
        KIND_EXPIRE,
        -remaining,
        None,
        grant.order_id,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Starts the points expiry sweep under the server's
/// supervision: a pass on the mail worker's interval,
/// draining on the shared shutdown watch. Balances exclude
/// expired grants whether or not the sweep has run — the
/// sweep exists so the ledger history shows the expiry as
/// a movement, not so the balance becomes right.
pub fn start_worker(pool: PgPool, mut shutdown: Receiver<bool>) -> tokio::task::JoinHandle<()> {
    let interval_secs = CONFIG.mail_worker_interval_secs.max(5);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
            }
            match expire_due(&pool).await {
                Ok(expired) if expired > 0 => {
                    tracing::info!(expired, "points expiry sweep expired grants");
                }
                Ok(_) => {}
                Err(e) => tracing::error!("points expiry pass failed: {e}"),
            }
        }
        tracing::info!("points expiry sweep stopped");
    });
    tracing::info!(interval_secs, "points expiry sweep started");
    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(id: i64, remaining: i64) -> GrantLot {
        GrantLot { id, remaining }
    }

    #[test]
    fn lines_single_rate_matches_order_level() {
        assert_eq!(earned_points_for_lines(&[(10_000, 1)], 0), 100);
        assert_eq!(earned_points_for_lines(&[(10_000, 1)], 500), 95);
        // Exact pro-rata shares: the per-line result equals
        // the order-level one.
        assert_eq!(
            earned_points_for_lines(&[(3_000, 5), (7_000, 5)], 1_000),
            450
        );
    }

    #[test]
    fn lines_mixed_rates_and_exclusion() {
        // 880 at 50% plus 6980 at the shop 1%.
        assert_eq!(earned_points_for_lines(&[(880, 50), (6_980, 1)], 0), 509);
        // A 0% line is points-excluded: it earns nothing.
        assert_eq!(earned_points_for_lines(&[(880, 0), (6_980, 1)], 0), 69);
    }

    #[test]
    fn lines_deduction_allocated_pro_rata() {
        // Deduction 500 over 880 + 6980: shares 55 and 443,
        // residual 2 to the largest line (445). Bases 825
        // and 6535 earn 412 + 65.
        assert_eq!(earned_points_for_lines(&[(880, 50), (6_980, 1)], 500), 477);
        // A deduction at or above the subtotal earns 0.
        assert_eq!(earned_points_for_lines(&[(880, 50)], 880), 0);
        assert_eq!(earned_points_for_lines(&[(880, 50)], 1_000), 0);
    }

    #[test]
    fn earn_is_percent_of_base_floored() {
        assert_eq!(earned_points(10_000, 1), 100);
        assert_eq!(earned_points(999, 1), 9);
        assert_eq!(earned_points(10_000, 5), 500);
        assert_eq!(earned_points(0, 1), 0);
        assert_eq!(earned_points(-500, 1), 0);
        assert_eq!(earned_points(10_000, 0), 0);
    }

    #[test]
    fn spend_plan_takes_fifo_across_lots() {
        let lots = [lot(1, 300), lot(2, 500), lot(3, 200)];
        assert_eq!(plan_spend(&lots, 700), Some(vec![(1, 300), (2, 400)]));
        assert_eq!(plan_spend(&lots, 300), Some(vec![(1, 300)]));
        assert_eq!(
            plan_spend(&lots, 1000),
            Some(vec![(1, 300), (2, 500), (3, 200)])
        );
    }

    #[test]
    fn spend_plan_refuses_when_lots_cannot_cover() {
        let lots = [lot(1, 300), lot(2, 500)];
        assert_eq!(plan_spend(&lots, 801), None);
        assert_eq!(plan_spend(&[], 1), None);
    }

    #[test]
    fn spend_plan_handles_zero_and_empty_lots() {
        assert_eq!(
            plan_spend(&[lot(1, 0), lot(2, 50)], 50),
            Some(vec![(2, 50)])
        );
        assert_eq!(plan_spend(&[lot(1, 100)], 0), Some(vec![]));
        assert_eq!(plan_spend(&[lot(1, 100)], -5), None);
    }
}
