//! Inquiry rules (Contact spec §3, §5): validation with
//! all errors at once, the two rate limits (per-IP in
//! memory, per-email in the DB), the acknowledgement mail
//! that never blocks a submission, and the admin-side
//! operations the inbox shares. Contents are personal data
//! and are never logged — actions log the inquiry id only.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::error::AppError;
use crate::mail;
use crate::messages::{
    BODY_REQUIRED_MESSAGE, EMAIL_FORMAT_MESSAGE, EMAIL_REQUIRED_MESSAGE, SUBJECT_REQUIRED_MESSAGE,
};
use crate::models;
use crate::repositories::inquiry_repository;
use crate::util::rate_limit_knobs::SWEEP_THRESHOLD;
use crate::validation::FieldError;
use crate::validation::is_valid_email;

/// The raw contact form, before validation.
pub struct InquiryInput {
    /// Name as typed.
    pub name: String,
    /// Email as typed.
    pub email: String,
    /// Category code as submitted (empty when unchosen).
    pub category: String,
    /// Order number as typed (may be empty).
    pub order_number: String,
    /// Body as typed.
    pub body: String,
}

/// Field length caps, counted in characters.
const NAME_MAX: usize = 50;
const EMAIL_MAX: usize = 254;
const ORDER_NUMBER_MAX: usize = 32;
const BODY_MAX: usize = 2000;
/// Links allowed in one body.
const LINK_MAX: usize = 3;

/// Validates the form, returning every error at once.
pub fn validate(input: &InquiryInput) -> Vec<FieldError> {
    let mut errors = Vec::new();
    let err = |field: &'static str, message: &str| FieldError {
        field,
        message: message.to_string(),
    };
    if input.name.trim().is_empty() {
        errors.push(err("name", "お名前を入力してください。"));
    } else if input.name.chars().count() > NAME_MAX {
        errors.push(err("name", "お名前は50文字以内で入力してください。"));
    }
    if input.email.trim().is_empty() {
        errors.push(err("email", EMAIL_REQUIRED_MESSAGE));
    } else if input.email.chars().count() > EMAIL_MAX || !is_valid_email(input.email.trim()) {
        errors.push(err("email", EMAIL_FORMAT_MESSAGE));
    }
    if models::category_for(&input.category).is_none() {
        errors.push(err("category", "お問い合わせ種別を選択してください。"));
    }
    if input.order_number.chars().count() > ORDER_NUMBER_MAX {
        errors.push(err(
            "order_number",
            "ご注文番号は32文字以内で入力してください。",
        ));
    }
    if input.body.trim().is_empty() {
        errors.push(err("body", "お問い合わせ内容を入力してください。"));
    } else if input.body.chars().count() > BODY_MAX {
        errors.push(err(
            "body",
            "お問い合わせ内容は2000文字以内で入力してください。",
        ));
    } else if count_links(&input.body) > LINK_MAX {
        errors.push(err(
            "body",
            "お問い合わせ内容に含められるリンクは3つまでです。",
        ));
    }
    errors
}

/// Counts link occurrences (`http://` + `https://`) in a
/// body, for the §5.3 cap.
fn count_links(body: &str) -> usize {
    body.matches("http://").count() + body.matches("https://").count()
}

/// Submission times per IP, for the rate limit.
static SUBMIT_LOG: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The per-IP window: 5 submissions per hour.
const SUBMIT_WINDOW: Duration = Duration::from_secs(3600);
const SUBMIT_LIMIT: usize = 5;

/// Whether one more submission from this IP is allowed,
/// recording it when it is. In-memory like the login
/// throttle: disposable by design.
fn ip_allowed(ip: &str) -> bool {
    let now = Instant::now();
    let mut log = SUBMIT_LOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if log.len() >= SWEEP_THRESHOLD && !log.contains_key(ip) {
        log.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < SUBMIT_WINDOW);
            !times.is_empty()
        });
    }
    let entry = log.entry(ip.to_string()).or_default();
    entry.retain(|t| now.duration_since(*t) < SUBMIT_WINDOW);
    if entry.len() >= SUBMIT_LIMIT {
        return false;
    }
    entry.push(now);
    true
}

/// The one rate-limit message for either limit — which
/// limit was hit is never identified.
pub const RATE_LIMIT_MESSAGE: &str =
    "短時間に多くのお問い合わせが送信されました。しばらく時間をおいてから再度お試しください。";

/// Why a submission did not go through.
pub enum SubmitError {
    /// A rate limit was hit (either kind).
    RateLimited,
    /// The database failed.
    Db(sqlx::Error),
}

/// Accepts one validated submission: rate checks, insert,
/// then the acknowledgement mail whose failure is logged
/// but never fails the submission. Returns
/// the new inquiry id.
///
/// # Errors
///
/// Returns [`SubmitError::RateLimited`] when either limit
/// is hit and [`SubmitError::Db`] when the insert fails.
pub async fn submit(
    pool: &PgPool,
    ip: &str,
    user_id: Option<i64>,
    input: &InquiryInput,
) -> Result<i64, SubmitError> {
    if !ip_allowed(ip) {
        return Err(SubmitError::RateLimited);
    }
    let email = input.email.trim();
    match inquiry_repository::count_by_email_last_day(pool, email).await {
        Ok(n) if n >= 3 => return Err(SubmitError::RateLimited),
        Ok(_) => {}
        Err(e) => return Err(SubmitError::Db(e)),
    }
    let order_number = {
        let trimmed = input.order_number.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    };
    let id = inquiry_repository::insert(
        pool,
        user_id,
        input.name.trim(),
        email,
        &input.category,
        order_number,
        input.body.trim(),
    )
    .await
    .map_err(SubmitError::Db)?;
    tracing::info!(
        action = "inquiry.create",
        inquiry_id = id,
        "inquiry accepted"
    );
    // The inbox row is the record; the acknowledgement is a
    // courtesy copy, so its outcome is reduced
    // to a flag before any further await (the mail error
    // type is not Send).
    let reference = reference_for(id);
    match mail::send_inquiry_acknowledgement(
        pool,
        email,
        &reference,
        models::category_label(&input.category),
        input.body.trim(),
        input.name.trim(),
    )
    .await
    {
        Ok(()) => {}
        Err(e) => {
            tracing::warn!(action = "inquiry.ack_mail", inquiry_id = id, error = %e, "acknowledgement mail failed");
        }
    }
    Ok(id)
}

/// Finds the real order a typed order number refers to,
/// for the admin detail's convenience link.
/// The typed text is parsed for its trailing id and the
/// loaded order's own number must match exactly — a
/// partial or foreign number links nothing.
/// Which fields the guarded admin save actually changed,
/// so the controller audits each under its own code.
pub struct AdminInquiryUpdate {
    /// The status moved.
    pub status_changed: bool,
    /// The memo text changed.
    pub memo_changed: bool,
}

/// Saves the admin status/memo form under the form-ticket
/// guard: the row is locked and its stamp compared with
/// the one the form was rendered from, so a second tab's
/// save can no longer silently revert the first tab's
/// status or memo. Only fields that actually changed are
/// written, as before.
pub async fn update_admin_fields(
    pool: &PgPool,
    id: i64,
    status: &str,
    memo: &str,
    expected: i64,
) -> Result<AdminInquiryUpdate, AppError> {
    let mut tx = pool.begin().await?;
    let Some(current) = inquiry_repository::find_for_update(&mut tx, id).await? else {
        return Err(AppError::NotFound);
    };
    crate::services::form_ticket_service::ensure_fresh(Some(current.updated_at), expected)?;
    let status_changed = crate::models::status_known(status) && status != current.status;
    if status_changed {
        inquiry_repository::update_status_tx(&mut tx, id, status).await?;
    }
    let memo_changed = memo != current.admin_memo;
    if memo_changed {
        inquiry_repository::update_memo_tx(&mut tx, id, memo).await?;
    }
    tx.commit().await?;
    Ok(AdminInquiryUpdate {
        status_changed,
        memo_changed,
    })
}

pub async fn match_order(pool: &PgPool, typed: &str) -> Option<models::Order> {
    let id: i64 = typed.rsplit('-').next()?.parse().ok()?;
    let order = crate::repositories::order_repository::find_by_id(pool, id)
        .await
        .ok()??;
    (order.order_number() == typed).then_some(order)
}

/// Validates a reply before it is previewed or sent.
pub fn validate_reply(subject: &str, body: &str) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if subject.trim().is_empty() {
        errors.push(FieldError {
            field: "subject",
            message: SUBJECT_REQUIRED_MESSAGE.to_string(),
        });
    } else if subject.chars().count() > 100 {
        errors.push(FieldError {
            field: "subject",
            message: "件名は100文字以内で入力してください。".to_string(),
        });
    }
    if body.trim().is_empty() {
        errors.push(FieldError {
            field: "body",
            message: BODY_REQUIRED_MESSAGE.to_string(),
        });
    } else if body.chars().count() > 4000 {
        errors.push(FieldError {
            field: "body",
            message: "本文は4000文字以内で入力してください。".to_string(),
        });
    }
    errors
}

/// Sends one reply and, only on success, stores it and
/// moves an open inquiry to in-progress.
/// Returns false when the mailer failed — nothing is
/// recorded then, so a reply that did not send is never
/// shown as sent.
///
/// # Errors
///
/// Returns an error only when storing a sent reply fails.
pub async fn send_reply(
    pool: &PgPool,
    inquiry: &models::Inquiry,
    admin_id: i64,
    subject: &str,
    body: &str,
) -> Result<bool, sqlx::Error> {
    // Same Send discipline as the acknowledgement: reduce
    // the mail outcome to a flag before the next await.
    let sent = match mail::send_inquiry_reply(&inquiry.email, subject, body).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(action = "inquiry.reply", inquiry_id = inquiry.id, error = %e, "reply mail failed");
            false
        }
    };
    if !sent {
        return Ok(false);
    }
    inquiry_repository::insert_reply(pool, inquiry.id, admin_id, subject, body).await?;
    if inquiry.status == "open" {
        inquiry_repository::update_status(pool, inquiry.id, "in_progress").await?;
    }
    Ok(true)
}

/// The customer-facing reference of one inquiry
/// (`INQ-000123`): the number the acknowledgement mail and
/// the member pages quote.
pub fn reference_for(id: i64) -> String {
    format!("INQ-{id:06}")
}

/// Validates the member order page's 返品・交換の申請 form:
/// a reason from the fixed catalogue and a free-text detail
/// under the inquiry body's own rules (required, character
/// cap, link cap), reported per field like every form.
pub fn validate_return_request(reason: &str, detail: &str) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if models::return_reason_for(reason).is_none() {
        errors.push(FieldError {
            field: "reason",
            message: "理由を選択してください。".to_string(),
        });
    }
    let detail = detail.trim();
    if detail.is_empty() {
        errors.push(FieldError {
            field: "detail",
            message: "詳細を入力してください。".to_string(),
        });
    } else if detail.chars().count() > BODY_MAX {
        errors.push(FieldError {
            field: "detail",
            message: "詳細は2000文字以内で入力してください。".to_string(),
        });
    } else if count_links(detail) > LINK_MAX {
        errors.push(FieldError {
            field: "detail",
            message: "詳細に含められるリンクは3つまでです。".to_string(),
        });
    }
    errors
}

/// Files one return/exchange request from the member order
/// page as an ordinary inquiry: category `returns`, the
/// order's own number in the order-number column (the
/// admin inbox links it to the order), and the reason at
/// the head of the body. Intake only — no refund or status
/// automation hangs off this; the operator answers from
/// the inbox like any inquiry. Assumes
/// [`validate_return_request`] passed.
/// One return/exchange request from the member order page,
/// bundled so the submit call stays readable.
pub struct ReturnRequest<'a> {
    /// The requesting member.
    pub user_id: i64,
    /// Their name (the order's delivery name).
    pub name: &'a str,
    /// Their account email.
    pub email: &'a str,
    /// The order the request is about (already proven theirs).
    pub order: &'a crate::models::Order,
    /// The reason code from the fixed catalogue.
    pub reason_code: &'a str,
    /// The free-text detail as typed.
    pub detail: &'a str,
}

pub async fn submit_return_request(
    pool: &PgPool,
    ip: &str,
    request: &ReturnRequest<'_>,
) -> Result<i64, SubmitError> {
    let label = models::return_reason_for(request.reason_code)
        .map(|reason| reason.label)
        .unwrap_or("その他");
    let body = format!(
        "【返品・交換の申請】\n理由：{label}\n\n{}",
        request.detail.trim()
    );
    let input = InquiryInput {
        name: request.name.to_string(),
        email: request.email.to_string(),
        category: "returns".to_string(),
        order_number: request.order.order_number(),
        body,
    };
    submit(pool, ip, Some(request.user_id), &input).await
}

// ---- Admin inbox reads ----

/// One inquiry by id for the admin screens. A read failure
/// logs and reads as missing: the screens redirect to the
/// inbox either way, but the failure stays visible.
pub async fn find_admin(pool: &PgPool, id: i64) -> Option<models::Inquiry> {
    inquiry_repository::find(pool, id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("inquiry lookup failed: {e}");
            None
        })
}

/// How many inquiries match the inbox list's status filter
/// (all when `None`). A failed read logs and reads as zero.
pub async fn count_filtered(pool: &PgPool, status: Option<&str>) -> i64 {
    inquiry_repository::count_filtered(pool, status)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("inquiry count failed: {e}");
            0
        })
}

/// One page of the admin inbox, newest first. A failed
/// read logs and reads as an empty page.
pub async fn list_page(
    pool: &PgPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Vec<models::Inquiry> {
    inquiry_repository::list_page(pool, status, limit, offset)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("inquiry list read failed: {e}");
            Vec::new()
        })
}

/// The four inbox chip counts: (all, open, in progress,
/// closed). Each failed read logs and reads as zero.
pub async fn chip_counts(pool: &PgPool) -> (i64, i64, i64, i64) {
    let count = |result: Result<i64, sqlx::Error>| {
        result.unwrap_or_else(|e| {
            tracing::error!("inquiry count failed: {e}");
            0
        })
    };
    (
        count(inquiry_repository::count_all(pool).await),
        count(inquiry_repository::count_by_status(pool, "open").await),
        count(inquiry_repository::count_by_status(pool, "in_progress").await),
        count(inquiry_repository::count_by_status(pool, "closed").await),
    )
}

/// One inquiry's reply thread. A failed read logs and
/// reads as no replies.
pub async fn replies(pool: &PgPool, inquiry_id: i64) -> Vec<models::InquiryReply> {
    inquiry_repository::replies_of(pool, inquiry_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("inquiry replies read failed: {e}");
            Vec::new()
        })
}

/// The member behind one inquiry, as (id, email) for the
/// detail page. A failed read logs and reads as no member.
pub async fn member_of(pool: &PgPool, user_id: i64) -> Option<(i64, String)> {
    crate::repositories::user_repository::find_by_id(pool, user_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("inquiry member lookup failed: {e}");
            None
        })
        .map(|u| (u.id, u.email))
}
