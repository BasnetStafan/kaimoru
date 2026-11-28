//! Admin ops status: the dashboard panel's figures.
//!
//! Display only. Unlike the dashboard's sales figures, every
//! value here degrades to an honest "unknown" label instead
//! of failing the page: a backup directory the process
//! cannot read (the ECS shape) or a stopped LLM must be
//! *visible* on this panel, not take the dashboard down.

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::repositories::{mail_repository, ops_repository, shipping_mail_repository};

/// The newest backup in the configured directory.
pub struct BackupInfo {
    /// File name of the newest `.sql` backup.
    pub filename: String,
    /// Its modification time, as a JST label.
    pub modified_label: String,
    /// Its size, human-readable.
    pub size_label: String,
}

/// Everything the ops status panel renders, pre-labelled.
pub struct OpsStatus {
    /// The newest backup, or `None` when none can be seen.
    pub backup: Option<BackupInfo>,
    /// Campaign deliveries waiting (queued or claimed).
    pub mail_waiting: i64,
    /// Campaign deliveries failed.
    pub mail_failed: i64,
    /// Campaign jobs currently claimed.
    pub mail_jobs_running: i64,
    /// Shipping mails waiting in the outbox.
    pub shipping_waiting: i64,
    /// Shipping mails that exhausted their attempts.
    pub shipping_failed: i64,
    /// When the payment sweep last finished a pass (JST
    /// label), or `None` when it never recorded one.
    pub sweep_last_run_label: Option<String>,
    /// Whether the LLM answered its health probe.
    pub llm_up: bool,
    /// The configured LLM model name.
    pub llm_model: String,
}

/// Reads the newest `.sql` file in the backup directory.
/// Synchronous directory IO on purpose: the directory is
/// tiny and local; a missing/unreadable directory is the
/// `None` the panel renders as 確認できません.
fn latest_backup() -> Option<BackupInfo> {
    let entries = std::fs::read_dir(&CONFIG.backup_dir).ok()?;
    let mut newest: Option<(String, std::time::SystemTime, u64)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".sql") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if newest.as_ref().is_none_or(|(_, t, _)| modified > *t) {
            newest = Some((name, modified, meta.len()));
        }
    }
    let (filename, modified, size) = newest?;
    let modified_label = chrono::DateTime::<chrono::Utc>::from(modified)
        .with_timezone(&crate::models::jst::jst())
        .format("%Y/%m/%d %H:%M")
        .to_string();
    let size_label = if size >= 1_000_000 {
        format!("{:.1} MB", size as f64 / 1_000_000.0)
    } else {
        format!("{} KB", size / 1_000)
    };
    Some(BackupInfo {
        filename,
        modified_label,
        size_label,
    })
}

/// Probes the LLM in the shape the configured endpoint speaks.
/// With an API key the endpoint is a cloud OpenAI-compatible API
/// (the key's presence is the cloud switch), whose liveness probe
/// is the authenticated model list — Ollama's `/api/tags` would
/// 404 there and report a healthy model as down. Without a key
/// the endpoint is the local Ollama server: the configured base
/// URL is its OpenAI-shaped `/v1` root, so the probe uses the
/// parent's `/api/tags`. Two seconds at most — the panel must
/// never hang on a dead model server.
async fn llm_up() -> bool {
    let base = CONFIG.llm_base_url.trim_end_matches('/');
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some(key) = CONFIG.llm_api_key() {
        let url = format!("{base}/models");
        return matches!(
            client.get(url).bearer_auth(key).send().await,
            Ok(resp) if resp.status().is_success()
        );
    }
    let root = base.strip_suffix("/v1").unwrap_or(base);
    let url = format!("{root}/api/tags");
    matches!(client.get(url).send().await, Ok(resp) if resp.status().is_success())
}

/// Assembles the panel's figures; each source falls back
/// independently (a failed count reads as zero only for the
/// mail figures, whose tables are the app's own — a failure
/// there also fails the dashboard's other reads upstream).
pub async fn load(pool: &PgPool) -> OpsStatus {
    let backup = latest_backup();
    let (mail_waiting, mail_failed) = mail_repository::delivery_backlog_counts(pool)
        .await
        .unwrap_or((0, 0));
    let mail_jobs_running = mail_repository::running_jobs_count(pool).await.unwrap_or(0);
    let shipping_waiting = shipping_mail_repository::count_waiting(pool)
        .await
        .unwrap_or(0);
    let shipping_failed = shipping_mail_repository::count_failed(pool)
        .await
        .unwrap_or(0);
    let sweep_last_run_label = ops_repository::last_job_run(pool, "payment_sweep")
        .await
        .ok()
        .flatten()
        .map(|at| {
            at.with_timezone(&crate::models::jst::jst())
                .format("%Y/%m/%d %H:%M")
                .to_string()
        });
    OpsStatus {
        backup,
        mail_waiting,
        mail_failed,
        mail_jobs_running,
        shipping_waiting,
        shipping_failed,
        sweep_last_run_label,
        llm_up: llm_up().await,
        llm_model: CONFIG.llm_model.clone(),
    }
}
