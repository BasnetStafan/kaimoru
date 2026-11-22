//! Step-mail engine (EC V2 #2; internal step-mail design record).
//!
//! The scheduling brain of scenario mail: it enrolls
//! members at registration (welcome), discovers delivered
//! orders (review request), and sends each due step
//! through the campaign pipeline's proven pieces — the
//! consent authority's `may_send`, the campaign renderer,
//! and the marketing transport. A scenario's sends are
//! marketing mail: consent is re-checked at the moment of
//! every step, and losing consent stops the run for good.

use crate::config::CONFIG;
use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::watch::Receiver;

use crate::error::AppError;
use crate::models::mail::EmailCampaign;
use crate::models::mail_scenario::MailScenarioEnrollment;
use crate::repositories::mail_repository;
use crate::repositories::scenario_repository;
use crate::repositories::user_repository;
use crate::services::mail_subscription_service;
use crate::services::marketing_mail_service;

/// The welcome series' machine key (migration 40).
pub const WELCOME_KEY: &str = "welcome";
/// The review-request scenario's machine key.
pub const REVIEW_REQUEST_KEY: &str = "review_request";
/// Consecutive send failures that end a run as `failed`;
/// mirrors the campaign pipeline's MAX_ATTEMPTS.
const MAX_ATTEMPTS: i32 = 3;
/// Enrollments processed per worker pass.
const DUE_BATCH: i64 = 50;
/// Delivered orders enrolled per worker pass.
const SCAN_BATCH: i64 = 50;

/// Enrolls a freshly registered member into the welcome
/// series, when the scenario exists and is enabled. Called
/// beside the consent record at registration; the caller
/// logs a failure rather than failing the sign-up.
pub async fn enroll_welcome(pool: &PgPool, user_id: i64) -> Result<(), AppError> {
    let Some(scenario) = scenario_repository::find_scenario_by_key(pool, WELCOME_KEY).await? else {
        return Ok(());
    };
    if !scenario.enabled {
        return Ok(());
    }
    let steps = scenario_repository::steps_for(pool, scenario.id).await?;
    let Some(first) = steps.first() else {
        return Ok(());
    };
    scenario_repository::insert_enrollment(pool, scenario.id, user_id, None, first.delay_days)
        .await?;
    Ok(())
}

/// The advisory-lock key of one scenario pass: a fixed
/// constant, defined exactly once, so every process agrees
/// on which lock serialises the engine (the house pattern;
/// the payment sweep, low-stock alerts and cart reminders
/// hold 741_001..=741_003).
const SCENARIO_LOCK_KEY: i64 = 741_004;

/// Runs one engine pass: the delivered-order trigger scan,
/// then every due step. Public for the suite-driven checks
/// and the worker alike.
///
/// One pass at a time across replicas: the engine claims no rows before sending, so
/// without the lock two worker processes sharing one
/// database would send the same due steps twice. The lock
/// is held on a dedicated connection for the pass's life;
/// not acquiring it is a normal outcome and skips quietly.
pub async fn run_once(pool: &PgPool) -> Result<(), AppError> {
    let mut lock_conn = pool
        .acquire()
        .await
        .map_err(|e| AppError::db("scenario lock acquire", e))?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(SCENARIO_LOCK_KEY)
        .fetch_one(&mut *lock_conn)
        .await
        .map_err(|e| AppError::db("scenario try lock", e))?;
    if !locked {
        tracing::debug!("step-mail pass skipped: another instance holds the lock");
        return Ok(());
    }
    let result = run_pass(pool).await;
    // A failed unlock leaves the session-scoped lock on
    // the connection: close it rather than returning it to
    // the pool, where it would stall later passes.
    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(SCENARIO_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await
    {
        tracing::error!("step-mail unlock failed; closing the lock connection: {e}");
        let _ = lock_conn.close().await;
    }
    result
}

/// The pass body, under the caller's advisory lock.
async fn run_pass(pool: &PgPool) -> Result<(), AppError> {
    enroll_delivered_orders(pool).await?;
    let due = scenario_repository::due_enrollments(pool, DUE_BATCH).await?;
    for enrollment in &due {
        if let Err(e) = process_enrollment(pool, enrollment).await {
            tracing::error!(
                enrollment_id = enrollment.id,
                "step-mail enrollment failed: {e}"
            );
        }
    }
    Ok(())
}

/// The order trigger: member orders newly seen in
/// `delivered` status enroll into the review-request
/// scenario, due after its first step's delay. The scan
/// guards on the order's `delivered_at` stamp inside a
/// thirty-day recency window:
/// delivery is the event the mail is about, and a
/// scenario re-enabled after a long pause must not mail
/// every historical buyer.
async fn enroll_delivered_orders(pool: &PgPool) -> Result<(), AppError> {
    let Some(scenario) =
        scenario_repository::find_scenario_by_key(pool, REVIEW_REQUEST_KEY).await?
    else {
        return Ok(());
    };
    if !scenario.enabled {
        return Ok(());
    }
    let steps = scenario_repository::steps_for(pool, scenario.id).await?;
    let Some(first) = steps.first() else {
        return Ok(());
    };
    let candidates =
        scenario_repository::delivered_orders_without_enrollment(pool, scenario.id, SCAN_BATCH)
            .await?;
    for (order_id, user_id) in candidates {
        scenario_repository::insert_enrollment(
            pool,
            scenario.id,
            user_id,
            Some(order_id),
            first.delay_days,
        )
        .await?;
    }
    Ok(())
}

/// Sends (or resolves) one enrollment's current step.
async fn process_enrollment(
    pool: &PgPool,
    enrollment: &MailScenarioEnrollment,
) -> Result<(), AppError> {
    let Some(step) =
        scenario_repository::step_at(pool, enrollment.scenario_id, enrollment.current_step).await?
    else {
        // The chain names a step that does not exist: the
        // run can never proceed, so end it as failed.
        let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
        scenario_repository::record_send(&mut tx, enrollment, "failed", Some("step missing"))
            .await?;
        scenario_repository::finish_enrollment(&mut tx, enrollment.id, "failed").await?;
        tx.commit().await.map_err(|e| AppError::db("commit", e))?;
        return Ok(());
    };

    // An order-triggered run sends only while its order
    // is still delivered: an order cancelled after
    // enrollment must not draw a review request. The run
    // ends quietly — nothing was attempted, so the send
    // log stays empty for it.
    if let Some(order_id) = enrollment.order_id {
        let order = crate::repositories::order_repository::find_by_id(pool, order_id).await?;
        if order.as_ref().map(|o| o.status.as_str()) != Some("delivered") {
            let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
            scenario_repository::finish_enrollment(&mut tx, enrollment.id, "stopped").await?;
            tx.commit().await.map_err(|e| AppError::db("commit", e))?;
            return Ok(());
        }
    }

    // Consent is the law of this mail system: re-checked at
    // the moment of the send, exactly like a campaign
    // delivery. No consent ends the run — it never resumes
    // silently if the member later re-subscribes.
    if !mail_subscription_service::may_send(pool, enrollment.user_id).await? {
        let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
        scenario_repository::record_send(&mut tx, enrollment, "skipped_consent", None).await?;
        scenario_repository::finish_enrollment(&mut tx, enrollment.id, "stopped").await?;
        tx.commit().await.map_err(|e| AppError::db("commit", e))?;
        return Ok(());
    }
    let Some(user) = user_repository::find_by_id(pool, enrollment.user_id).await? else {
        let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
        scenario_repository::finish_enrollment(&mut tx, enrollment.id, "stopped").await?;
        tx.commit().await.map_err(|e| AppError::db("commit", e))?;
        return Ok(());
    };
    let Some(subscription) = mail_repository::find_subscription(pool, enrollment.user_id).await?
    else {
        // may_send held, so a subscription row exists; a
        // vanished one is treated as the consent loss it is.
        let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
        scenario_repository::record_send(&mut tx, enrollment, "skipped_consent", None).await?;
        scenario_repository::finish_enrollment(&mut tx, enrollment.id, "stopped").await?;
        tx.commit().await.map_err(|e| AppError::db("commit", e))?;
        return Ok(());
    };

    // Claim the run before any mail moves: the claim leases the run for ten
    // minutes and re-checks the scenario is still
    // enabled. If a previous pass already sent this step
    // but failed before advancing, the send log holds the
    // proof and the run advances without a second send.
    if scenario_repository::send_already_recorded(pool, enrollment).await? {
        advance_after_send(pool, enrollment).await?;
        return Ok(());
    }
    if !scenario_repository::claim_enrollment(pool, enrollment.id).await? {
        return Ok(());
    }

    // The step's content rides the campaign renderer as a
    // transient one-recipient campaign: {order_url} is the
    // engine's own variable, substituted first.
    let order_url = enrollment.order_id.map(|id| {
        format!(
            "{}/mypage/orders/{}",
            CONFIG
                .public_base_url()
                .map(|base| base.trim_end_matches('/').to_string())
                .unwrap_or_default(),
            id
        )
    });
    let campaign = EmailCampaign {
        id: 0,
        name: format!("step-mail:{}", enrollment.scenario_id),
        template_id: None,
        subject: substitute_order_url(&step.subject, order_url.as_deref()),
        body_html: substitute_order_url(&step.body_html, order_url.as_deref()),
        body_text: substitute_order_url(&step.body_text, order_url.as_deref()),
        audience: "subscribed".to_string(),
        segment_id: None,
        status: "sent".to_string(),
        scheduled_at: None,
        sent_at: None,
        created_by: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let member_name = format!("{} {}", user.family_name, user.given_name);
    let rendered = match marketing_mail_service::render(
        pool,
        &campaign,
        &member_name,
        &marketing_mail_service::unsubscribe_url(&subscription.unsubscribe_token),
    )
    .await
    {
        Ok(rendered) => rendered,
        Err(e) => {
            // A render failure is a failed attempt like a
            // transport failure: counted, retried, and
            // terminal at the cap — never a silent wedge
            // that retries every pass forever.
            note_failure(pool, enrollment, &e.to_string()).await?;
            return Ok(());
        }
    };
    let send_result = crate::mail::send_marketing(
        &user.email,
        &rendered.subject,
        &rendered.html,
        &rendered.text,
    )
    .await
    .map_err(|e| e.to_string());

    match send_result {
        Ok(()) => {
            // The send is recorded in its own transaction
            // FIRST: if the advance
            // then fails, the next pass finds this row and
            // advances without sending again. A failure of
            // this single-row insert is the only remaining
            // duplicate window, and it needs the database
            // itself to be failing at that moment.
            let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
            scenario_repository::record_send(&mut tx, enrollment, "sent", None).await?;
            tx.commit().await.map_err(|e| AppError::db("commit", e))?;
            advance_after_send(pool, enrollment).await?;
        }
        Err(message) => {
            note_failure(pool, enrollment, &message).await?;
        }
    }
    Ok(())
}

/// Moves a run past a step whose send is durably
/// recorded: on to the next step after its delay, or to
/// `completed` when the chain ends. Shared by the normal
/// post-send path and the recovery path that finds the
/// send already logged.
async fn advance_after_send(
    pool: &PgPool,
    enrollment: &MailScenarioEnrollment,
) -> Result<(), AppError> {
    let steps = scenario_repository::steps_for(pool, enrollment.scenario_id).await?;
    let next = steps
        .iter()
        .find(|s| s.step_no == enrollment.current_step + 1);
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    match next {
        Some(next_step) => {
            scenario_repository::advance_enrollment(
                &mut tx,
                enrollment.id,
                next_step.step_no,
                next_step.delay_days,
            )
            .await?;
        }
        None => {
            scenario_repository::finish_enrollment(&mut tx, enrollment.id, "completed").await?;
        }
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// Counts one failed attempt on a run's current step;
/// at the cap the failure is recorded and the run ends
/// as `failed`, so a broken step can never wedge a member
/// or spin the worker.
async fn note_failure(
    pool: &PgPool,
    enrollment: &MailScenarioEnrollment,
    message: &str,
) -> Result<(), AppError> {
    let attempts = scenario_repository::note_send_failure(pool, enrollment.id).await?;
    // Every failed attempt lands in the send log, not only
    // the terminal one: an ongoing
    // mail outage must be visible in the admin counts
    // while it is happening, not just after a run dies.
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    scenario_repository::record_send(&mut tx, enrollment, "failed", Some(message)).await?;
    if attempts >= MAX_ATTEMPTS {
        scenario_repository::finish_enrollment(&mut tx, enrollment.id, "failed").await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// Substitutes the engine's `{order_url}` variable; steps
/// without an order keep the token out of their copy, and
/// any stray token renders empty rather than leaking.
fn substitute_order_url(body: &str, order_url: Option<&str>) -> String {
    body.replace("{order_url}", order_url.unwrap_or_default())
}

/// Starts the step-mail worker: one engine pass per mail
/// worker interval, until shutdown. Mirrors the points
/// expiry worker's shape.
pub fn start_worker(pool: PgPool, mut shutdown: Receiver<bool>) -> tokio::task::JoinHandle<()> {
    let interval_secs = CONFIG.mail_worker_interval_secs.max(5);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
            }
            if let Err(e) = run_once(&pool).await {
                tracing::error!("step-mail pass failed: {e}");
            }
        }
        tracing::info!("step-mail worker stopped");
    });
    tracing::info!(interval_secs, "step-mail worker started");
    handle
}
