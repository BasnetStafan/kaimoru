//! Admin identity and audit-trail queries.
//!
//! Owns the `admins` and `admin_audit_logs` tables: every
//! write goes through here (one read exception —
//! `content_page_repository` joins `admins` for editor names).
//! The audit log is append-only by construction: this module
//! exposes an insert and filtered reads, and deliberately no
//! update or delete path.

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Admin, AdminIdentity, AdminSummary, AuditLogRow};

/// Every column of an `admins` row, named — never `SELECT *`, so
/// a schema change cannot silently change what a query loads.
const ADMIN_COLUMNS: &str = "id, email, password_hash, display_name, role, permissions, is_active, created_at, last_login_at";

/// The identity columns of an `admins` row: everything except
/// the password hash, which only the auth reads may load.
const ADMIN_IDENTITY_COLUMNS: &str = "id, email, display_name, role, permissions, is_active";

/// The columns of an `admins` row the management screens read:
/// the identity plus status and timestamps, never the hash.
const ADMIN_SUMMARY_COLUMNS: &str =
    "id, email, display_name, role, permissions, is_active, created_at, last_login_at";

/// Finds an admin by (already normalised) email, for the login
/// flow. Returns the password hash: authentication use only.
pub async fn find_admin_by_email(pool: &PgPool, email: &str) -> Result<Option<Admin>, AppError> {
    sqlx::query_as::<_, Admin>(&format!(
        "SELECT {ADMIN_COLUMNS} FROM admins WHERE email = $1"
    ))
    .bind(email)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find admin by email", e))
}

/// Finds an admin by ID, for the `CurrentAdmin` extractor. Loads
/// the identity only — never the password hash: this runs on
/// every admin request, and the hash has no business travelling.
pub async fn find_admin_by_id(pool: &PgPool, id: i64) -> Result<Option<AdminIdentity>, AppError> {
    sqlx::query_as::<_, AdminIdentity>(&format!(
        "SELECT {ADMIN_IDENTITY_COLUMNS} FROM admins WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find admin by id", e))
}

/// Finds an admin by ID with the password hash, for the settings
/// page's password change and the CSRF credential fingerprint —
/// the only by-ID read that may load it. Authentication use
/// only, like [`find_admin_by_email`].
pub async fn find_admin_auth_by_id(pool: &PgPool, id: i64) -> Result<Option<Admin>, AppError> {
    sqlx::query_as::<_, Admin>(&format!("SELECT {ADMIN_COLUMNS} FROM admins WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("find admin auth by id", e))
}

/// Replaces one admin's password hash (the settings page's change,
/// after the service verified the current password and the new
/// one's strength). Only the hash is written — never the password
/// itself, never logged.
pub async fn update_admin_password(
    pool: &PgPool,
    admin_id: i64,
    password_hash: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE admins SET password_hash = $2 WHERE id = $1")
        .bind(admin_id)
        .bind(password_hash)
        .execute(pool)
        .await
        .map_err(|e| AppError::db("update admin password", e))?;
    Ok(())
}

/// Replaces one admin's display name (the settings page's profile
/// form), written exactly as validated by the service.
pub async fn update_admin_display_name(
    pool: &PgPool,
    admin_id: i64,
    display_name: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE admins SET display_name = $2 WHERE id = $1")
        .bind(admin_id)
        .bind(display_name)
        .execute(pool)
        .await
        .map_err(|e| AppError::db("update admin display name", e))?;
    Ok(())
}

/// How many admin accounts exist. The env bootstrap creates the
/// first admin only while this is zero — exactly once, and it
/// never resets an existing admin's password.
pub async fn count_admins(pool: &PgPool) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM admins")
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count admins", e))
}

/// Creates one admin account (the env bootstrap's insert). The
/// bootstrap creates the FIRST admin, who must land as `owner`:
/// the column default is `staff`, which would leave a fresh
/// deployment with no owner at all (every owner-gated surface
/// unreachable). Accounts created later through the admin UI go
/// through `insert_admin_account` with an explicit role instead.
pub async fn insert_admin(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
    display_name: &str,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO admins (email, password_hash, display_name, role) \
         VALUES ($1, $2, $3, 'owner') RETURNING id",
    )
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("insert admin", e))
}

/// Stamps the last-login time after a successful admin login.
pub async fn touch_last_login(pool: &PgPool, admin_id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE admins SET last_login_at = now() WHERE id = $1")
        .bind(admin_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::db("touch admin last login", e))?;
    Ok(())
}

/// Lists every admin account for the management screen,
/// oldest first (the bootstrap owner is row one).
pub async fn list_admins(pool: &PgPool) -> Result<Vec<AdminSummary>, AppError> {
    sqlx::query_as::<_, AdminSummary>(&format!(
        "SELECT {ADMIN_SUMMARY_COLUMNS} FROM admins ORDER BY id"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list admins", e))
}

/// Finds one admin account for the management edit screen.
pub async fn find_admin_summary(pool: &PgPool, id: i64) -> Result<Option<AdminSummary>, AppError> {
    sqlx::query_as::<_, AdminSummary>(&format!(
        "SELECT {ADMIN_SUMMARY_COLUMNS} FROM admins WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find admin summary", e))
}

/// Creates one admin account inside the caller's transaction
/// (the management create, audited by the caller in the same
/// transaction). Only the hash is written — never a password.
pub async fn insert_admin_account(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    email: &str,
    password_hash: &str,
    display_name: &str,
    role: &str,
    permissions: &[String],
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO admins (email, password_hash, display_name, role, permissions) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .bind(role)
    .bind(permissions)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert admin account", e))
}

/// The admin account's current `updated_at`: the stamp
/// an edit form ticket is minted with at render time.
/// (The `AdminSummary` row struct does not carry the
/// stamp, so it is read on its own, like the tag's.)
pub async fn updated_at(
    pool: &PgPool,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM admins WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(value)
}

/// Returns the admin account's current `updated_at` with
/// the row locked: the edit form's staleness guard
/// compares its form ticket's stamp against this value
/// inside the update transaction (the product guard's
/// shape).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM admins WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Replaces one admin account's name, role, permissions and
/// active flag inside the caller's transaction (the management
/// edit, audited by the caller in the same transaction). The
/// email is the sign-in ID and never changes here; the password
/// has its own write path.
pub async fn update_admin_account(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    display_name: &str,
    role: &str,
    permissions: &[String],
    is_active: bool,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE admins SET display_name = $2, role = $3, permissions = $4, is_active = $5 \
         WHERE id = $1",
    )
    .bind(admin_id)
    .bind(display_name)
    .bind(role)
    .bind(permissions)
    .bind(is_active)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update admin account", e))?;
    Ok(())
}

/// The OTHER active owners' ids, locked for the caller's
/// transaction: the last-owner guard counts these, and the
/// row locks make a concurrent demotion or deactivation of
/// one of them wait — two edits at once can no longer each
/// see the other owner still standing and both go through.
pub async fn active_owner_ids_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    excluding_id: i64,
) -> Result<Vec<i64>, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM admins WHERE role = 'owner' AND is_active AND id <> $1 FOR UPDATE",
    )
    .bind(excluding_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| AppError::db("lock active owners", e))
}

/// Appends one audit row. Called by the admin services inside
/// the same transaction as the mutation it records, so an
/// action and its record commit or roll back together.
/// IDs and action names only — never personal data.
pub async fn insert_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    action: &str,
    target_type: &str,
    target_id: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO admin_audit_logs (admin_id, action, target_type, target_id) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(admin_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert audit log", e))?;
    Ok(())
}

/// Appends one audit row per target ID in one statement: the bulk
/// status change writes a row per changed product, and looping
/// the single-row insert cost a round trip each inside the
/// transaction. Values are always bound — only placeholder
/// numbers are composed. Same append-only discipline as
/// Appends one audit row for a MEMBER actor (member
/// order cancellation): `admin_id` stays NULL and
/// `actor_user_id` carries the member, so the trail can
/// answer "who cancelled this?" for member cancels too.
/// Same all-or-nothing rule as [`insert_audit`].
pub async fn insert_audit_member(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    action: &str,
    target_type: &str,
    target_id: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO admin_audit_logs (admin_id, actor_user_id, action, target_type, target_id) \
         VALUES (NULL, $1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .execute(&mut **tx)
    .await
    .map(|_| ())
    .map_err(|e| AppError::db("insert member audit", e))
}

/// [`insert_audit`].
pub async fn insert_audit_many(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    action: &str,
    target_type: &str,
    target_ids: &[i64],
) -> Result<(), AppError> {
    if target_ids.is_empty() {
        return Ok(());
    }
    let groups: Vec<String> = (0..target_ids.len())
        .map(|i| format!("($1, $2, $3, ${})", i + 4))
        .collect();
    let sql = format!(
        "INSERT INTO admin_audit_logs (admin_id, action, target_type, target_id) \
         VALUES {}",
        groups.join(", ")
    );
    let mut query = sqlx::query(&sql)
        .bind(admin_id)
        .bind(action)
        .bind(target_type);
    for id in target_ids {
        query = query.bind(id);
    }
    query
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("insert audit logs (bulk)", e))?;
    Ok(())
}

/// Appends one audit row outside a mutation transaction — used
/// for the admin login event, which mutates no business record.
pub async fn insert_audit_standalone(
    pool: &PgPool,
    admin_id: i64,
    action: &str,
    target_type: &str,
    target_id: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO admin_audit_logs (admin_id, action, target_type, target_id) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(admin_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("insert audit log (standalone)", e))?;
    Ok(())
}

/// Returns one page of the audit viewer, newest first, with the
/// total row count. Every filter value comes from the viewer's
/// fixed option lists or a parsed date, and is always bound. The
/// date range compares the row's JST calendar day, so the end
/// date includes its whole day.
pub async fn list_audit_logs(
    pool: &PgPool,
    target_type: Option<&str>,
    action: Option<&str>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AuditLogRow>, i64), AppError> {
    let rows = sqlx::query_as::<_, AuditLogWithTotal>(
        "SELECT l.id, l.admin_id, COALESCE(a.display_name, '会員') AS admin_name, l.action, \
                l.target_type, l.target_id, l.created_at, \
                COUNT(*) OVER () AS total \
         FROM admin_audit_logs l \
         LEFT JOIN admins a ON a.id = l.admin_id \
         WHERE ($1::text IS NULL OR l.target_type = $1) \
           AND ($2::text IS NULL OR l.action = $2) \
           AND ($3::date IS NULL \
                OR (l.created_at AT TIME ZONE 'Asia/Tokyo')::date >= $3) \
           AND ($4::date IS NULL \
                OR (l.created_at AT TIME ZONE 'Asia/Tokyo')::date <= $4) \
         ORDER BY l.created_at DESC, l.id DESC \
         LIMIT $5 OFFSET $6",
    )
    .bind(target_type)
    .bind(action)
    .bind(date_from)
    .bind(date_to)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list audit logs", e))?;
    // A page past the end returns no rows to carry the total;
    // only then is a separate count worth its round trip.
    if rows.is_empty() && offset > 0 {
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM admin_audit_logs l \
             WHERE ($1::text IS NULL OR l.target_type = $1) \
               AND ($2::text IS NULL OR l.action = $2) \
               AND ($3::date IS NULL \
                    OR (l.created_at AT TIME ZONE 'Asia/Tokyo')::date >= $3) \
               AND ($4::date IS NULL \
                    OR (l.created_at AT TIME ZONE 'Asia/Tokyo')::date <= $4)",
        )
        .bind(target_type)
        .bind(action)
        .bind(date_from)
        .bind(date_to)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count audit logs", e))?;
        return Ok((Vec::new(), total));
    }
    Ok(with_total(rows))
}

/// Returns every audit row recorded against one target, newest
/// first — the order detail's 変更履歴.
pub async fn list_audit_logs_for_target(
    pool: &PgPool,
    target_type: &str,
    target_id: i64,
) -> Result<Vec<AuditLogRow>, AppError> {
    sqlx::query_as::<_, AuditLogRow>(
        "SELECT l.id, l.admin_id, COALESCE(a.display_name, '会員') AS admin_name, l.action, \
                l.target_type, l.target_id, l.created_at \
         FROM admin_audit_logs l \
         LEFT JOIN admins a ON a.id = l.admin_id \
         WHERE l.target_type = $1 AND l.target_id = $2 \
         ORDER BY l.created_at DESC, l.id DESC",
    )
    .bind(target_type)
    .bind(target_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list audit logs for target", e))
}

/// Splits the window-function total off audit rows.
fn with_total(rows: Vec<AuditLogWithTotal>) -> (Vec<AuditLogRow>, i64) {
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    (
        rows.into_iter()
            .map(|r| AuditLogRow {
                id: r.id,
                admin_id: r.admin_id,
                admin_name: r.admin_name,
                action: r.action,
                target_type: r.target_type,
                target_id: r.target_id,
                created_at: r.created_at,
            })
            .collect(),
        total,
    )
}

/// [`AuditLogRow`] plus the pagination total, before splitting.
#[derive(sqlx::FromRow)]
struct AuditLogWithTotal {
    id: i64,
    admin_id: Option<i64>,
    admin_name: String,
    action: String,
    target_type: String,
    target_id: Option<i64>,
    created_at: chrono::DateTime<chrono::Utc>,
    total: i64,
}
