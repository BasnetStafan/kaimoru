//! Step-mail (scenario mail) data (EC V2 #2,
//! internal step-mail design record): the scenario definitions, their
//! steps, one member's run (enrollment) and the send log.
//! Content lives on the step rows; sends reuse the campaign
//! pipeline's consent check, renderer and transport.

use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// One automated mail sequence (a row of `mail_scenarios`).
#[derive(Debug, Clone, FromRow)]
pub struct MailScenario {
    /// Surrogate ID.
    pub id: i64,
    /// Stable machine key the code enrolls by (`welcome`,
    /// `review_request`).
    pub key: String,
    /// Operator-facing name.
    pub name: String,
    /// Operator-facing description of the sequence.
    pub description: String,
    /// `signup` or `order_delivered`.
    pub trigger_kind: String,
    /// The admin toggle: a disabled scenario sends nothing
    /// and skips nothing; its due enrollments wait.
    pub enabled: bool,
    /// When the scenario was seeded.
    pub created_at: DateTime<Utc>,
    /// When the scenario last changed.
    pub updated_at: DateTime<Utc>,
}

/// One mail in a scenario's chain (a row of
/// `mail_scenario_steps`).
#[derive(Debug, Clone, FromRow)]
pub struct MailScenarioStep {
    /// Surrogate ID.
    pub id: i64,
    /// The owning scenario.
    pub scenario_id: i64,
    /// 1-based position in the chain.
    pub step_no: i32,
    /// Days after the previous step; for step 1, days
    /// after enrollment.
    pub delay_days: i32,
    /// Subject line (may carry the renderer's variables).
    pub subject: String,
    /// HTML body (may carry `{name}`, `{order_url}` and
    /// `{{product:<id>}}` blocks).
    pub body_html: String,
    /// Plain-text body.
    pub body_text: String,
}

/// One member's run through a scenario (a row of
/// `mail_scenario_enrollments`).
#[derive(Debug, Clone, FromRow)]
pub struct MailScenarioEnrollment {
    /// Surrogate ID.
    pub id: i64,
    /// The scenario being run.
    pub scenario_id: i64,
    /// The enrolled member.
    pub user_id: i64,
    /// The triggering order, for order-triggered
    /// scenarios; `None` otherwise.
    pub order_id: Option<i64>,
    /// The step due next (1-based).
    pub current_step: i32,
    /// Consecutive send failures on the current step.
    pub attempts: i32,
    /// When the current step comes due.
    pub next_due_at: DateTime<Utc>,
    /// `active`, `completed`, `stopped` or `failed`.
    pub status: String,
    /// When the run began.
    pub created_at: DateTime<Utc>,
    /// When the run last changed.
    pub updated_at: DateTime<Utc>,
}

/// Per-scenario send totals, for the admin list.
#[derive(Debug, Clone, FromRow)]
pub struct ScenarioSendCounts {
    /// The scenario the totals belong to.
    pub scenario_id: i64,
    /// Steps successfully sent.
    pub sent: i64,
    /// Steps skipped for lack of consent.
    pub skipped: i64,
    /// Steps that exhausted their send attempts.
    pub failed: i64,
}
