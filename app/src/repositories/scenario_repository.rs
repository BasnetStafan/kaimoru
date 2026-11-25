//! Step-mail persistence (EC V2 #2): scenario and step
//! reads, idempotent enrollment, the due queue, and the
//! send log. All statements are parameterised; the
//! enrollment inserts lean on the partial unique indexes
//! from migration 40, so retries and worker restarts can
//! never enroll a member (or an order) twice.

use sqlx::{PgPool, Postgres, Transaction};

use crate::models::mail_scenario::{
    MailScenario, MailScenarioEnrollment, MailScenarioStep, ScenarioSendCounts,
};

/// Scenario columns, in model order.
const SCENARIO_COLUMNS: &str =
    "id, key, name, description, trigger_kind, enabled, created_at, updated_at";
/// Step columns, in model order.
const STEP_COLUMNS: &str = "id, scenario_id, step_no, delay_days, subject, body_html, body_text";
/// One scenario by its machine key.
pub async fn find_scenario_by_key(
    pool: &PgPool,
    key: &str,
) -> Result<Option<MailScenario>, sqlx::Error> {
    sqlx::query_as::<_, MailScenario>(&format!(
        "SELECT {SCENARIO_COLUMNS} FROM mail_scenarios WHERE key = $1"
    ))
    .bind(key)
    .fetch_optional(pool)
    .await
}

/// Every scenario, in seed order — the admin list.
pub async fn list_scenarios(pool: &PgPool) -> Result<Vec<MailScenario>, sqlx::Error> {
    sqlx::query_as::<_, MailScenario>(&format!(
        "SELECT {SCENARIO_COLUMNS} FROM mail_scenarios ORDER BY id"
    ))
    .fetch_all(pool)
    .await
}

/// A scenario's steps in chain order.
pub async fn steps_for(
    pool: &PgPool,
    scenario_id: i64,
) -> Result<Vec<MailScenarioStep>, sqlx::Error> {
    sqlx::query_as::<_, MailScenarioStep>(&format!(
        "SELECT {STEP_COLUMNS} FROM mail_scenario_steps \
         WHERE scenario_id = $1 ORDER BY step_no"
    ))
    .bind(scenario_id)
    .fetch_all(pool)
    .await
}

/// One step of a scenario, by position.
pub async fn step_at(
    pool: &PgPool,
    scenario_id: i64,
    step_no: i32,
) -> Result<Option<MailScenarioStep>, sqlx::Error> {
    sqlx::query_as::<_, MailScenarioStep>(&format!(
        "SELECT {STEP_COLUMNS} FROM mail_scenario_steps \
         WHERE scenario_id = $1 AND step_no = $2"
    ))
    .bind(scenario_id)
    .bind(step_no)
    .fetch_optional(pool)
    .await
}

/// Flips a scenario's admin toggle.
pub async fn set_scenario_enabled(
    pool: &PgPool,
    scenario_id: i64,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mail_scenarios SET enabled = $2, updated_at = now() WHERE id = $1")
        .bind(scenario_id)
        .bind(enabled)
        .execute(pool)
        .await?;
    Ok(())
}

/// Enrolls a member (or an order) into a scenario, due
/// after `first_delay_days`. Idempotent: a duplicate run
/// is silently not created. Returns whether a row was
/// inserted.
pub async fn insert_enrollment(
    pool: &PgPool,
    scenario_id: i64,
    user_id: i64,
    order_id: Option<i64>,
    first_delay_days: i32,
) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(
        "INSERT INTO mail_scenario_enrollments \
         (scenario_id, user_id, order_id, next_due_at) \
         VALUES ($1, $2, $3, now() + make_interval(days => $4)) \
         ON CONFLICT DO NOTHING",
    )
    .bind(scenario_id)
    .bind(user_id)
    .bind(order_id)
    .bind(first_delay_days)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(rows > 0)
}

/// Member orders in `delivered` status that have no
/// enrollment for the scenario yet — the trigger scan's
/// candidates, oldest first. Only orders placed after the
/// scenario existed qualify: without that guard, shipping
/// this feature would enroll the shop's whole delivered
/// history on the first pass and mail every past buyer.
pub async fn delivered_orders_without_enrollment(
    pool: &PgPool,
    scenario_id: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT o.id, o.user_id FROM orders o \
         JOIN mail_scenarios s ON s.id = $1 \
         WHERE o.status = 'delivered' AND o.user_id IS NOT NULL \
           AND o.delivered_at >= s.created_at \
           AND o.delivered_at >= now() - interval '30 days' \
           AND NOT EXISTS (SELECT 1 FROM mail_scenario_enrollments e \
                           WHERE e.scenario_id = $1 AND e.order_id = o.id) \
         ORDER BY o.id LIMIT $2",
    )
    .bind(scenario_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Active enrollments whose current step is due, for
/// enabled scenarios only — a disabled scenario's queue
/// waits untouched. Oldest due first.
pub async fn due_enrollments(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<MailScenarioEnrollment>, sqlx::Error> {
    sqlx::query_as::<_, MailScenarioEnrollment>(
        "SELECT e.id, e.scenario_id, e.user_id, e.order_id, e.current_step, \
                e.attempts, e.next_due_at, e.status, e.created_at, e.updated_at \
         FROM mail_scenario_enrollments e \
         JOIN mail_scenarios s ON s.id = e.scenario_id \
         WHERE e.status = 'active' AND e.next_due_at <= now() AND s.enabled \
         ORDER BY e.next_due_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Whether the send log already holds a `sent` row for
/// the run's current step: the recovery signal for a pass
/// that sent the mail but failed before advancing the run
///.
pub async fn send_already_recorded(
    pool: &PgPool,
    enrollment: &MailScenarioEnrollment,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM mail_scenario_sends \
         WHERE enrollment_id = $1 AND step_no = $2 AND status = 'sent')",
    )
    .bind(enrollment.id)
    .bind(enrollment.current_step)
    .fetch_one(pool)
    .await
}

/// Claims one due run for this pass by pushing its due
/// time out by a ten-minute lease, so a crash or a failed
/// write after a successful send cannot make the next
/// pass send the same step again right away. The claim
/// also re-checks that the scenario is still enabled, so
/// a scenario disabled mid-pass sends nothing further
///. Returns false when the run is
/// no longer claimable (advanced, ended, or disabled).
pub async fn claim_enrollment(pool: &PgPool, enrollment_id: i64) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE mail_scenario_enrollments e \
         SET next_due_at = now() + interval '10 minutes', updated_at = now() \
         WHERE e.id = $1 AND e.status = 'active' AND e.next_due_at <= now() \
           AND EXISTS (SELECT 1 FROM mail_scenarios s \
                       WHERE s.id = e.scenario_id AND s.enabled)",
    )
    .bind(enrollment_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected == 1)
}

/// Appends one outcome to the send log.
pub async fn record_send(
    tx: &mut Transaction<'_, Postgres>,
    enrollment: &MailScenarioEnrollment,
    status: &str,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mail_scenario_sends \
         (enrollment_id, scenario_id, user_id, step_no, status, error) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(enrollment.id)
    .bind(enrollment.scenario_id)
    .bind(enrollment.user_id)
    .bind(enrollment.current_step)
    .bind(status)
    .bind(error)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Advances a run to its next step, due after
/// `delay_days`, clearing the failure counter.
pub async fn advance_enrollment(
    tx: &mut Transaction<'_, Postgres>,
    enrollment_id: i64,
    next_step: i32,
    delay_days: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE mail_scenario_enrollments \
         SET current_step = $2, attempts = 0, \
             next_due_at = now() + make_interval(days => $3), updated_at = now() \
         WHERE id = $1",
    )
    .bind(enrollment_id)
    .bind(next_step)
    .bind(delay_days)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Ends a run: `completed`, `stopped` or `failed`.
pub async fn finish_enrollment(
    tx: &mut Transaction<'_, Postgres>,
    enrollment_id: i64,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE mail_scenario_enrollments SET status = $2, updated_at = now() WHERE id = $1",
    )
    .bind(enrollment_id)
    .bind(status)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Counts one more consecutive send failure on a run and
/// returns the new total.
pub async fn note_send_failure(pool: &PgPool, enrollment_id: i64) -> Result<i32, sqlx::Error> {
    // A failure also defers the next attempt (whole-app
    // review B2): +1 hour on the first strike, +6 hours on
    // the second, so a mail outage of a few minutes cannot
    // burn all three strikes inside a couple of worker
    // passes. The third strike ends the run as failed (the
    // caller's cap check), where the deferral no longer
    // matters.
    sqlx::query_scalar::<_, i32>(
        "UPDATE mail_scenario_enrollments \
         SET attempts = attempts + 1, \
             next_due_at = now() + CASE WHEN attempts + 1 >= 2 \
                                        THEN interval '6 hours' \
                                        ELSE interval '1 hour' END, \
             updated_at = now() WHERE id = $1 \
         RETURNING attempts",
    )
    .bind(enrollment_id)
    .fetch_one(pool)
    .await
}

/// Send totals per scenario, for the admin list.
pub async fn send_counts(pool: &PgPool) -> Result<Vec<ScenarioSendCounts>, sqlx::Error> {
    sqlx::query_as::<_, ScenarioSendCounts>(
        "SELECT scenario_id, \
                COUNT(*) FILTER (WHERE status = 'sent') AS sent, \
                COUNT(*) FILTER (WHERE status = 'skipped_consent') AS skipped, \
                COUNT(*) FILTER (WHERE status = 'failed') AS failed \
         FROM mail_scenario_sends GROUP BY scenario_id",
    )
    .fetch_all(pool)
    .await
}
