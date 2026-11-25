//! Auth service.
//!
//! The business rules of the member account lifecycle:
//! sign-up and login, profile and credential changes, the
//! withdrawal code, SNS sign-in completion and session
//! revocation — verification codes and input validation
//! throughout. Controllers own HTTP; this layer owns the
//! rules. Password
//! mechanics live in [`super::password`], the shared field rules
//! in [`crate::validation`], and every failure is the shared
//! [`AppError`].

use chrono::{Duration, Utc};
use rand::Rng;
use sqlx::PgPool;

use crate::error::AppError;
use crate::mail;
use crate::messages::{
    ACCOUNT_CREATE_FAILED_MESSAGE, ADDRESS_LINE_REQUIRED_MESSAGE, ALREADY_VERIFIED_MESSAGE,
    CITY_REQUIRED_MESSAGE, CURRENT_PASSWORD_MISMATCH_MESSAGE,
    EMAIL_ALREADY_REGISTERED_LOGIN_MESSAGE, EMAIL_ALREADY_REGISTERED_MESSAGE, EMAIL_FORMAT_MESSAGE,
    FAMILY_NAME_REQUIRED_MESSAGE, GIVEN_NAME_REQUIRED_MESSAGE, LOGIN_FAILED_MESSAGE,
    MAIL_SEND_FAILED_MESSAGE, PASSWORD_SAME_AS_CURRENT_MESSAGE, POSTAL_CODE_FORMAT_MESSAGE,
    PREFECTURE_REQUIRED_MESSAGE, REGISTRATION_INCOMPLETE_MESSAGE,
    VERIFICATION_CODE_ATTEMPTS_EXHAUSTED_MESSAGE, VERIFICATION_CODE_EXPIRED_MESSAGE,
    VERIFICATION_CODE_INCOMPLETE_MESSAGE, VERIFICATION_CODE_RESEND_LIMIT_MESSAGE,
    verification_code_mismatch_attempts_message, verification_code_resend_wait_message,
};

use crate::models::{CodeChallenge, PendingRegistration, User};
use crate::repositories::challenge_repository::ChallengeTable;
use crate::repositories::{
    challenge_repository, pending_registration_repository, session_revocation_repository,
    user_repository,
};
use crate::services::password;
use crate::validation::{self, FieldError};

/// How long a verification code stays valid.
const CODE_LIFETIME_MINUTES: i64 = 10;
/// Wrong entries allowed before a code is invalidated.
const MAX_CODE_ATTEMPTS: i32 = 5;
/// Seconds to wait before a code may be resent.
const RESEND_COOLDOWN_SECONDS: i64 = 60;
/// How many times a code may be resent for one sign-up.
const MAX_RESENDS: i32 = 5;
/// Coupon issued to every new member at sign-up completion.
pub const WELCOME_COUPON_CODE: &str = "WELCOME500";

/// The result of an auth operation. Failures are [`AppError`]:
/// business refusals ride in `Conflict` with their user-safe
/// explanation, validation in `Validation`, database failures in
/// `Db` — never a bare display string.
pub type AuthResult<T> = Result<T, AppError>;

/// A business refusal with its user-safe explanation.
fn refused(message: impl Into<String>) -> AppError {
    AppError::Conflict(message.into())
}

/// Maps a database failure to [`AppError::Db`], logging the
/// details here: the log is where they belong, not the screen.
fn db_error(e: sqlx::Error) -> AppError {
    AppError::db("auth database error", e)
}

/// Starts a sign-up: validates the email, stores a pending registration
/// holding only the email and the code hash, and sends the code.
///
/// # Privacy
///
/// The code is generated here, hashed before storage, and passed only
/// to the mail sender. It is never logged.
/// Returns the recipient parts a code mail's context
/// takes from an account: display name, kana name and
/// phone, in the mail sender's argument order.
fn mail_name_parts(user: &crate::models::User) -> (String, String, String) {
    (
        format!("{} {}", user.family_name, user.given_name),
        format!("{} {}", user.family_name_kana, user.given_name_kana),
        user.phone.clone(),
    )
}

pub async fn start_signup(pool: &PgPool, email: &str) -> AuthResult<i64> {
    let email = email.trim().to_lowercase();
    if !validation::is_valid_email(&email) {
        return Err(refused(EMAIL_FORMAT_MESSAGE));
    }

    // Housekeeping: abandoned sign-ups past their grace day are
    // deleted here, the one place new sign-ups begin.
    let _ = pending_registration_repository::delete_expired(pool).await;

    let code = generate_code();
    let code_hash = hash_code(&code, &email);
    let expires_at = Utc::now() + Duration::minutes(CODE_LIFETIME_MINUTES);

    let pending_id = pending_registration_repository::create(pool, &email, &code_hash, expires_at)
        .await
        .map_err(db_error)?;

    mail::send_verification_code(pool, &email, &code, "", "", "")
        .await
        .map_err(|_| refused(MAIL_SEND_FAILED_MESSAGE))?;

    // Only the pending ID and action are logged: never the email or code.
    tracing::info!(pending_id, action = "signup_started", "auth action");
    Ok(pending_id)
}

/// Resends a fresh code for a pending registration, enforcing the
/// cooldown and the resend limit from the spec.
pub async fn resend_code(pool: &PgPool, pending: &PendingRegistration) -> AuthResult<()> {
    if pending.verified {
        return Err(refused(ALREADY_VERIFIED_MESSAGE));
    }
    let waited = (Utc::now() - pending.code_sent_at).num_seconds();
    if waited < RESEND_COOLDOWN_SECONDS {
        return Err(refused(verification_code_resend_wait_message(
            RESEND_COOLDOWN_SECONDS - waited,
        )));
    }
    if pending.resend_count >= MAX_RESENDS {
        return Err(refused(VERIFICATION_CODE_RESEND_LIMIT_MESSAGE));
    }

    let code = generate_code();
    let code_hash = hash_code(&code, &pending.email);
    let expires_at = Utc::now() + Duration::minutes(CODE_LIFETIME_MINUTES);
    pending_registration_repository::replace_code(pool, pending.id, &code_hash, expires_at)
        .await
        .map_err(db_error)?;
    mail::send_verification_code(pool, &pending.email, &code, "", "", "")
        .await
        .map_err(|_| refused(MAIL_SEND_FAILED_MESSAGE))?;

    tracing::info!(
        pending_id = pending.id,
        action = "code_resent",
        "auth action"
    );
    Ok(())
}

/// Checks an entered code against a pending registration.
/// Wrong entries are counted in the database; after the limit, the
/// code is dead even if it is later typed correctly.
pub async fn verify_code(
    pool: &PgPool,
    pending: &PendingRegistration,
    entered: &str,
) -> AuthResult<()> {
    if pending.verified {
        return Ok(());
    }
    if Utc::now() > pending.expires_at {
        return Err(refused(VERIFICATION_CODE_EXPIRED_MESSAGE));
    }
    if pending.attempts >= MAX_CODE_ATTEMPTS {
        return Err(refused(VERIFICATION_CODE_ATTEMPTS_EXHAUSTED_MESSAGE));
    }

    if crate::session::constant_time_eq(
        &hash_code(entered.trim(), &pending.email),
        &pending.code_hash,
    ) {
        pending_registration_repository::mark_verified(pool, pending.id)
            .await
            .map_err(db_error)?;
        tracing::info!(
            pending_id = pending.id,
            action = "code_verified",
            "auth action"
        );
        Ok(())
    } else {
        // Atomic with the limit in SQL: a parallel request cannot
        // spend from the same attempts snapshot.
        let counted = pending_registration_repository::record_failed_attempt_capped(
            pool,
            pending.id,
            MAX_CODE_ATTEMPTS,
        )
        .await
        .map_err(db_error)?;
        if !counted {
            return Err(refused(VERIFICATION_CODE_ATTEMPTS_EXHAUSTED_MESSAGE));
        }
        let remaining = MAX_CODE_ATTEMPTS - pending.attempts - 1;
        Err(refused(verification_code_mismatch_attempts_message(
            remaining,
        )))
    }
}

/// The member details submitted at the details step, before validation.
pub struct DetailsForm {
    /// Plain password, hashed inside this layer and never stored raw.
    pub password: String,
    /// Family name in kanji.
    pub family_name: String,
    /// Given name in kanji.
    pub given_name: String,
    /// Family name in katakana.
    pub family_name_kana: String,
    /// Given name in katakana.
    pub given_name_kana: String,
    /// Birth date text (YYYY-MM-DD), optional.
    pub birth_date: String,
    /// Gender text, optional.
    pub gender: String,
    /// Postal code, hyphens tolerated.
    pub postal_code: String,
    /// Prefecture name.
    pub prefecture: String,
    /// City and ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building name, optional.
    pub building: String,
    /// Phone number, hyphens tolerated.
    pub phone: String,
    /// Whether the mail magazine was accepted.
    pub mail_magazine: bool,
}

/// Saves the member details onto the pending registration, with
/// the password hashed by Argon2id before it touches the
/// database. Validation failure is [`AppError::Validation`]
/// with every failing field; anything else is a form-level
/// refusal.
pub async fn save_details(
    pool: &PgPool,
    pending: &PendingRegistration,
    form: &DetailsForm,
) -> AuthResult<()> {
    if !pending.verified {
        return Err(refused(VERIFICATION_CODE_INCOMPLETE_MESSAGE));
    }
    let field_errors = validate_details(form);
    if !field_errors.is_empty() {
        return Err(AppError::Validation(field_errors));
    }

    let password_hash = password::hash_password_async(form.password.clone()).await?;
    store_details(pool, pending, form, Some(password_hash)).await
}

/// Saves the member details for an SNS registration: the same
/// validation as [`save_details`] minus the password rule, and
/// no password hash is stored - the account is created
/// passwordless and its identity is linked at completion.
pub async fn save_details_sns(
    pool: &PgPool,
    pending: &PendingRegistration,
    form: &DetailsForm,
) -> AuthResult<()> {
    if !pending.verified {
        return Err(refused(VERIFICATION_CODE_INCOMPLETE_MESSAGE));
    }
    let field_errors = validate_details_fields(form, false);
    if !field_errors.is_empty() {
        return Err(AppError::Validation(field_errors));
    }
    store_details(pool, pending, form, None).await
}

/// The storage half of the details steps, shared by the email
/// and SNS variants: everything after validation and the
/// (optional) password hash.
async fn store_details(
    pool: &PgPool,
    pending: &PendingRegistration,
    form: &DetailsForm,
    password_hash: Option<String>,
) -> AuthResult<()> {
    // A malformed birth date is that field's error, not a banner.
    let birth_date = parse_birth_date(&form.birth_date).map_err(|message| {
        AppError::Validation(vec![FieldError {
            field: "birth_date",
            message,
        }])
    })?;
    let gender = empty_to_none(&form.gender);
    let building = empty_to_none(&form.building);
    // Validation has already proven both digit rules, so the
    // shared helpers cannot refuse here; the fallback only keeps
    // the types honest.
    let postal_code = validation::postal_code_digits(&form.postal_code).unwrap_or_default();
    let phone = validation::phone_digits(&form.phone).unwrap_or_default();

    pending_registration_repository::save_details(
        pool,
        pending.id,
        password_hash.as_deref(),
        form.family_name.trim(),
        form.given_name.trim(),
        form.family_name_kana.trim(),
        form.given_name_kana.trim(),
        birth_date,
        gender.as_deref(),
        &postal_code,
        form.prefecture.trim(),
        form.city.trim(),
        form.address_line.trim(),
        building.as_deref(),
        &phone,
        form.mail_magazine,
    )
    .await
    .map_err(db_error)?;
    Ok(())
}

/// Completes a sign-up: creates the user, grants the welcome
/// coupon unless this email has redeemed one before, and
/// removes the pending registration, as one transaction.
pub async fn complete_signup(
    pool: &PgPool,
    pending: &PendingRegistration,
    cart_session_key: Option<&str>,
) -> AuthResult<User> {
    if !pending.verified || pending.password_hash.is_none() {
        return Err(refused(REGISTRATION_INCOMPLETE_MESSAGE));
    }
    if user_repository::exists_by_email(pool, &pending.email)
        .await
        .map_err(db_error)?
    {
        return Err(refused(EMAIL_ALREADY_REGISTERED_LOGIN_MESSAGE));
    }

    // The D4 once-per-email-ever rule (Account Deletion
    // spec): a returner whose withdrawn predecessor already
    // redeemed WELCOME500 is not granted it again; one who
    // never redeemed it still is.
    let welcome_code =
        if crate::services::withdraw_service::welcome_previously_redeemed(pool, &pending.email)
            .await?
        {
            None
        } else {
            Some(WELCOME_COUPON_CODE)
        };
    let user_id =
        user_repository::create_from_pending(pool, pending, welcome_code, cart_session_key)
            .await
            .map_err(db_error)?;
    let user = user_repository::find_by_id(pool, user_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| refused(ACCOUNT_CREATE_FAILED_MESSAGE))?;

    tracing::info!(user_id, action = "signup_completed", "auth action");
    // The marketing consent choice becomes the subscription
    // record of record. A failure here never
    // fails the registration — the safe direction is "no
    // row = unsubscribed".
    if let Err(e) = crate::services::mail_subscription_service::record_signup(
        pool,
        user.id,
        pending.mail_magazine,
    )
    .await
    {
        tracing::error!("mail subscription record failed for user {}: {e}", user.id);
    }
    // Step-mail: the welcome series enrolls at the same
    // moment (EC V2 #2). A failure here never fails the
    // registration either — the member simply never
    // enters the series.
    if let Err(e) = crate::services::scenario_service::enroll_welcome(pool, user.id).await {
        tracing::error!("welcome enrollment failed for user {}: {e}", user.id);
    }
    Ok(user)
}

/// Completes an SNS sign-up: the provider already verified
/// the email, so the pending row carries no password hash; the
/// account is created passwordless with the provider identity
/// linked in the same transaction. The welcome-coupon
/// once-per-email rule matches [`complete_signup`].
pub async fn complete_signup_sns(
    pool: &PgPool,
    pending: &PendingRegistration,
    cart_session_key: Option<&str>,
    provider: &str,
    provider_user_id: &str,
    provider_email: Option<&str>,
) -> AuthResult<User> {
    if !pending.verified || pending.password_hash.is_some() {
        return Err(refused(REGISTRATION_INCOMPLETE_MESSAGE));
    }
    if user_repository::exists_by_email(pool, &pending.email)
        .await
        .map_err(db_error)?
    {
        return Err(refused(EMAIL_ALREADY_REGISTERED_LOGIN_MESSAGE));
    }

    let welcome_code =
        if crate::services::withdraw_service::welcome_previously_redeemed(pool, &pending.email)
            .await?
        {
            None
        } else {
            Some(WELCOME_COUPON_CODE)
        };
    // The password column is NOT NULL, but nobody can ever
    // sign in with this hash: it is of a random secret that is
    // generated here and never stored or shown anywhere.
    let random_secret: String = (0..32)
        .map(|_| format!("{:x}", rand::thread_rng().gen_range(0..16)))
        .collect();
    let random_hash = password::hash_password_async(random_secret).await?;
    let user_id = user_repository::create_from_pending_sns(
        pool,
        pending,
        welcome_code,
        cart_session_key,
        &random_hash,
        provider,
        provider_user_id,
        provider_email,
    )
    .await
    .map_err(db_error)?;
    let user = user_repository::find_by_id(pool, user_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| refused(ACCOUNT_CREATE_FAILED_MESSAGE))?;

    tracing::info!(user_id, action = "sns_signup_completed", "auth action");
    // The marketing consent choice becomes the subscription
    // record of record. A failure here never
    // fails the registration — the safe direction is "no
    // row = unsubscribed".
    if let Err(e) = crate::services::mail_subscription_service::record_signup(
        pool,
        user.id,
        pending.mail_magazine,
    )
    .await
    {
        tracing::error!("mail subscription record failed for user {}: {e}", user.id);
    }
    // Step-mail: the welcome series enrolls at the same
    // moment (EC V2 #2). A failure here never fails the
    // registration either — the member simply never
    // enters the series.
    if let Err(e) = crate::services::scenario_service::enroll_welcome(pool, user.id).await {
        tracing::error!("welcome enrollment failed for user {}: {e}", user.id);
    }
    Ok(user)
}

/// Checks an email and password pair for login.
/// The failure message is identical for unknown email and wrong
/// password, so the response never reveals which one was wrong.
pub async fn login(pool: &PgPool, email: &str, password: &str) -> AuthResult<User> {
    let failure = || refused(LOGIN_FAILED_MESSAGE);
    let email = email.trim().to_lowercase();

    let Some((user_id, stored_hash)) = user_repository::password_hash_for_email(pool, &email)
        .await
        .map_err(db_error)?
    else {
        // Timing equalization: run the same Argon2 verification
        // against a dummy hash, so the response time does not
        // reveal that this email has no account.
        let _ = password::verify_password_async(
            password.to_string(),
            password::dummy_hash().to_string(),
        )
        .await;
        return Err(failure());
    };
    if !password::verify_password_async(password.to_string(), stored_hash).await {
        return Err(failure());
    }
    let user = user_repository::find_by_id(pool, user_id)
        .await
        .map_err(db_error)?
        .ok_or_else(failure)?;
    tracing::info!(user_id, action = "login", "auth action");
    Ok(user)
}

/// Loads a pending registration owned by the current flow, if present.
pub async fn load_pending(pool: &PgPool, pending_id: i64) -> AuthResult<PendingRegistration> {
    pending_registration_repository::find_by_id(pool, pending_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| refused("登録の有効期限が切れています。はじめからやり直してください。"))
}

/// Loads one member by ID, for pages that re-read the profile
/// after an update. A database failure is logged and reads as
/// absence; the caller falls back to what it already holds.
pub async fn load_user(pool: &PgPool, user_id: i64) -> Option<User> {
    match user_repository::find_by_id(pool, user_id).await {
        Ok(user) => user,
        Err(e) => {
            tracing::error!(user_id, "user load failed: {e}");
            None
        }
    }
}

/// Generates a 6 digit code with a secure random source.
/// Leading zeros are valid, so the code is handled as text throughout.
fn generate_code() -> String {
    let n: u32 = rand::thread_rng().gen_range(0..1_000_000);
    format!("{n:06}")
}

/// Hashes a code with HMAC-SHA-256, keyed by a server secret and bound
/// to the email, so a leaked table alone cannot be brute forced offline
/// without the secret, and a code never works for another address.
fn hash_code(code: &str, email: &str) -> String {
    // Fail fast: without this secret a leaked code_hash table
    // could be brute-forced offline, so the app must not start
    // without it. Startup validation in the server binary checks
    // the variable before any sign-up can reach this; local
    // run.sh exports the development value.
    static SECRET: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        std::env::var("VERIFICATION_CODE_SECRET")
            .expect("VERIFICATION_CODE_SECRET must be set (see README / run.sh)")
    });
    let secret: &str = &SECRET;
    // The HMAC itself is the shared helper (the webhook verifies
    // with the same code); this function owns only the secret
    // and the email binding.
    crate::util::hmac_util::hmac_sha256_hex(
        secret.as_bytes(),
        &[email.as_bytes(), b":", code.as_bytes()],
    )
}

/// Validates the details form and returns one error per failing
/// field (the first failing rule for that field), so the form can
/// show every problem in a single pass.
fn validate_details(form: &DetailsForm) -> Vec<FieldError> {
    validate_details_fields(form, true)
}

/// Validates the profile edit form, which has no password field —
/// the same field validation as the details form.
pub fn validate_profile(form: &DetailsForm) -> Vec<FieldError> {
    validate_details_fields(form, false)
}

/// The shared body of the details/profile validation. The digit
/// and kana rules live in [`crate::validation`], shared with
/// checkout; this function only attaches each form's messages.
fn validate_details_fields(form: &DetailsForm, include_password: bool) -> Vec<FieldError> {
    let mut errors: Vec<FieldError> = Vec::new();
    let mut push = |field: &'static str, message: &str| {
        errors.push(FieldError {
            field,
            message: message.to_string(),
        });
    };

    if include_password && let Some(message) = password::password_rule_error(&form.password) {
        push("password", message);
    }
    if form.family_name.trim().is_empty() {
        push("family_name", FAMILY_NAME_REQUIRED_MESSAGE);
    }
    if form.given_name.trim().is_empty() {
        push("given_name", GIVEN_NAME_REQUIRED_MESSAGE);
    }
    if !validation::is_katakana(form.family_name_kana.trim()) {
        push("family_name_kana", "セイは全角カタカナで入力してください。");
    }
    if !validation::is_katakana(form.given_name_kana.trim()) {
        push("given_name_kana", "メイは全角カタカナで入力してください。");
    }
    if validation::postal_code_digits(&form.postal_code).is_none() {
        push("postal_code", POSTAL_CODE_FORMAT_MESSAGE);
    }
    if form.prefecture.trim().is_empty() {
        push("prefecture", PREFECTURE_REQUIRED_MESSAGE);
    }
    if form.city.trim().is_empty() {
        push("city", CITY_REQUIRED_MESSAGE);
    }
    if form.address_line.trim().is_empty() {
        push("address_line", ADDRESS_LINE_REQUIRED_MESSAGE);
    }
    if validation::phone_digits(&form.phone).is_none() {
        push(
            "phone",
            "電話番号は10桁または11桁の数字で入力してください。",
        );
    }
    errors
}

/// Parses the optional birth date field.
fn parse_birth_date(text: &str) -> Result<Option<chrono::NaiveDate>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map(Some)
        .map_err(|_| "生年月日はYYYY-MM-DD形式で入力してください。".to_string())
}

/// Trims a string and maps an empty result to None.
fn empty_to_none(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Updates a member's profile from the mypage edit form. Same
/// validation and normalisation as sign-up: postal code and phone
/// are stored digits-only, an empty building becomes NULL.
///
/// A database failure is [`AppError::Db`]: it must surface as a
/// generic form banner, never disguised as a field error on data
/// the member entered correctly.
///
/// `expected` is the form-ticket staleness guard: the
/// `updated_at` (epoch microseconds) the edit form was
/// rendered from, recorded server-side by
/// `form_ticket_service` when the form was minted. If the
/// row has moved since (another tab saved first), the save
/// refuses with [`AppError::Conflict`] rather than silently
/// reverting the newer values — above all the mail-magazine
/// consent that rides the same submit.
pub async fn update_profile(
    pool: &PgPool,
    user_id: i64,
    form: &DetailsForm,
    expected: i64,
) -> AuthResult<()> {
    let errors = validate_profile(form);
    if !errors.is_empty() {
        return Err(AppError::Validation(errors));
    }
    let building = form.building.trim();
    let update = crate::repositories::user_repository::ProfileUpdate {
        family_name: form.family_name.trim().to_string(),
        given_name: form.given_name.trim().to_string(),
        family_name_kana: form.family_name_kana.trim().to_string(),
        given_name_kana: form.given_name_kana.trim().to_string(),
        postal_code: validation::postal_code_digits(&form.postal_code).unwrap_or_default(),
        prefecture: form.prefecture.trim().to_string(),
        city: form.city.trim().to_string(),
        address_line: form.address_line.trim().to_string(),
        building: (!building.is_empty()).then(|| building.to_string()),
        phone: validation::phone_digits(&form.phone).unwrap_or_default(),
        mail_magazine: form.mail_magazine,
    };
    // The mail magazine toggle rides the same submit, and
    // the subscription table is the record of record
    // the users row and the consent record
    // commit in ONE transaction. Splitting them would let a
    // failed sync silently lose an opt-out while sending
    // keeps consulting the subscription table — the one
    // direction a consent write must never fail quietly.
    let mut tx = pool.begin().await.map_err(db_error)?;
    // The form-ticket staleness guard (see
    // `form_ticket_service`).
    let current = crate::repositories::user_repository::updated_at_for_update(&mut tx, user_id)
        .await
        .map_err(db_error)?;
    crate::services::form_ticket_service::ensure_fresh(current, expected)?;
    crate::repositories::user_repository::update_profile(&mut tx, user_id, &update)
        .await
        .map_err(db_error)?;
    crate::services::mail_subscription_service::sync_preference_tx(
        &mut tx,
        user_id,
        form.mail_magazine,
    )
    .await?;
    tx.commit().await.map_err(db_error)?;
    Ok(())
}

/// Changes a member's password: the current password must verify
/// against the stored Argon2id hash, the new one must pass the
/// shared rule and differ from the old.
///
/// On success the member's other sessions are revoked: the
/// session that made the change (`keep_session_id`) keeps
/// working; every other session is signed out, so a session
/// stolen before the change cannot survive it.
pub async fn change_password(
    pool: &PgPool,
    user_id: i64,
    current: &str,
    new: &str,
    keep_session_id: Option<String>,
) -> AuthResult<()> {
    let stored = crate::repositories::user_repository::password_hash_for_id(pool, user_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| refused("パスワードの変更に失敗しました。"))?;
    if !password::verify_password_async(current.to_string(), stored).await {
        return Err(refused(CURRENT_PASSWORD_MISMATCH_MESSAGE));
    }
    if current == new {
        return Err(refused(PASSWORD_SAME_AS_CURRENT_MESSAGE));
    }
    if let Some(message) = password::password_rule_error(new) {
        return Err(refused(message));
    }
    let hash = password::hash_password_async(new.to_string()).await?;
    crate::repositories::user_repository::update_password_hash(pool, user_id, &hash)
        .await
        .map_err(db_error)?;
    revoke_sessions(pool, user_id, keep_session_id.as_deref()).await;
    Ok(())
}

/// Sets the first password of a passwordless (SNS-registered)
/// account: no current password exists to check, so the rule
/// check and the session revocation are all that carries over
/// from [`change_password`].
pub async fn set_first_password(
    pool: &PgPool,
    user_id: i64,
    new: &str,
    keep_session_id: Option<String>,
) -> AuthResult<()> {
    if let Some(message) = password::password_rule_error(new) {
        return Err(refused(message));
    }
    let hash = password::hash_password_async(new.to_string()).await?;
    user_repository::set_password(pool, user_id, &hash)
        .await
        .map_err(db_error)?;
    revoke_sessions(pool, user_id, keep_session_id.as_deref()).await;
    tracing::info!(user_id, action = "password_set", "auth action");
    Ok(())
}

/// Starts the withdrawal proof for a passwordless account: a
/// code challenge to the account's own email, under the same
/// rules as the other challenges. Returns the challenge ID.
pub async fn start_withdrawal_code(pool: &PgPool, user: &User) -> AuthResult<i64> {
    // Housekeeping, as at every flow start.
    let _ = challenge_repository::delete_expired(pool, ChallengeTable::WithdrawalCodes).await;

    let code = generate_code();
    let code_hash = hash_code(&code, &user.email);
    let expires_at = Utc::now() + Duration::minutes(CODE_LIFETIME_MINUTES);
    let challenge_id = challenge_repository::create(
        pool,
        ChallengeTable::WithdrawalCodes,
        user.id,
        &user.email,
        &code_hash,
        expires_at,
    )
    .await
    .map_err(db_error)?;
    let (name, kana, phone) = mail_name_parts(user);
    mail::send_withdrawal_code(pool, &user.email, &code, &name, &kana, &phone)
        .await
        .map_err(|_| refused(MAIL_SEND_FAILED_MESSAGE))?;

    tracing::info!(
        user_id = user.id,
        challenge_id,
        action = "withdrawal_code_started",
        "auth action"
    );
    Ok(challenge_id)
}

/// Deletes a member's sessions from the session store, keeping
/// `keep_session_id` when given.
///
/// A revocation failure is logged, never raised: the password
/// change it follows has already committed, and reporting failure
/// would tell the member something untrue (their new password is
/// in effect). The session store shares this database, so a store
/// that cannot be written is one no session can be loaded from
/// either.
async fn revoke_sessions(pool: &PgPool, user_id: i64, keep_session_id: Option<&str>) {
    match session_revocation_repository::delete_sessions_for_user(pool, user_id, keep_session_id)
        .await
    {
        Ok(count) => {
            tracing::info!(
                user_id,
                revoked = count,
                action = "sessions_revoked",
                "auth action"
            );
        }
        Err(e) => {
            tracing::error!(user_id, "session revocation failed: {e}");
        }
    }
}

/// Revokes a member's other sessions after a credential-state
/// change that is not itself a password change (the two-factor
/// disable): the same revocation the password change runs,
/// shared rather than re-implemented.
pub async fn revoke_other_sessions(pool: &PgPool, user_id: i64, keep_session_id: Option<String>) {
    revoke_sessions(pool, user_id, keep_session_id.as_deref()).await;
}

/// Loads a code challenge owned by the current flow, if present.
pub async fn load_challenge(
    pool: &PgPool,
    table: ChallengeTable,
    challenge_id: i64,
) -> AuthResult<CodeChallenge> {
    challenge_repository::find_by_id(pool, table, challenge_id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| refused("手続きの有効期限が切れています。はじめからやり直してください。"))
}

/// Checks an entered code against a code challenge (password
/// reset or email change). The rules are the sign-up code's
/// rules: expiry first, the attempt limit counted atomically in
/// the database, and a constant-time hash comparison.
pub async fn verify_challenge(
    pool: &PgPool,
    table: ChallengeTable,
    challenge: &CodeChallenge,
    entered: &str,
) -> AuthResult<()> {
    if challenge.verified {
        return Ok(());
    }
    if Utc::now() > challenge.expires_at {
        return Err(refused(VERIFICATION_CODE_EXPIRED_MESSAGE));
    }
    if challenge.attempts >= MAX_CODE_ATTEMPTS {
        return Err(refused(VERIFICATION_CODE_ATTEMPTS_EXHAUSTED_MESSAGE));
    }

    if crate::session::constant_time_eq(
        &hash_code(entered.trim(), &challenge.email),
        &challenge.code_hash,
    ) {
        challenge_repository::mark_verified(pool, table, challenge.id)
            .await
            .map_err(db_error)?;
        tracing::info!(
            challenge_id = challenge.id,
            action = "challenge_code_verified",
            "auth action"
        );
        Ok(())
    } else {
        // Atomic with the limit in SQL, as for sign-up: a parallel
        // request cannot spend from the same snapshot.
        let counted = challenge_repository::record_failed_attempt_capped(
            pool,
            table,
            challenge.id,
            MAX_CODE_ATTEMPTS,
        )
        .await
        .map_err(db_error)?;
        if !counted {
            return Err(refused(VERIFICATION_CODE_ATTEMPTS_EXHAUSTED_MESSAGE));
        }
        let remaining = MAX_CODE_ATTEMPTS - challenge.attempts - 1;
        Err(refused(verification_code_mismatch_attempts_message(
            remaining,
        )))
    }
}

/// Resends a fresh code for a code challenge, enforcing the
/// cooldown and the resend limit from the sign-up spec. The
/// challenge's table decides which mail is sent.
pub async fn resend_challenge(
    pool: &PgPool,
    table: ChallengeTable,
    challenge: &CodeChallenge,
) -> AuthResult<()> {
    if challenge.verified {
        return Err(refused(ALREADY_VERIFIED_MESSAGE));
    }
    let waited = (Utc::now() - challenge.code_sent_at).num_seconds();
    if waited < RESEND_COOLDOWN_SECONDS {
        return Err(refused(verification_code_resend_wait_message(
            RESEND_COOLDOWN_SECONDS - waited,
        )));
    }
    if challenge.resend_count >= MAX_RESENDS {
        return Err(refused(VERIFICATION_CODE_RESEND_LIMIT_MESSAGE));
    }

    let code = generate_code();
    let code_hash = hash_code(&code, &challenge.email);
    let expires_at = Utc::now() + Duration::minutes(CODE_LIFETIME_MINUTES);
    challenge_repository::replace_code(pool, table, challenge.id, &code_hash, expires_at)
        .await
        .map_err(db_error)?;
    let (name, kana, phone) = match user_repository::find_by_id(pool, challenge.user_id)
        .await
        .ok()
        .flatten()
    {
        Some(user) => mail_name_parts(&user),
        None => (String::new(), String::new(), String::new()),
    };
    let sent = match table {
        ChallengeTable::PasswordResets => {
            mail::send_password_reset_code(pool, &challenge.email, &code, &name, &kana, &phone)
                .await
        }
        ChallengeTable::EmailChanges => {
            mail::send_email_change_code(pool, &challenge.email, &code, &name, &kana, &phone).await
        }
        ChallengeTable::WithdrawalCodes => {
            mail::send_withdrawal_code(pool, &challenge.email, &code, &name, &kana, &phone).await
        }
    };
    sent.map_err(|_| refused(MAIL_SEND_FAILED_MESSAGE))?;

    tracing::info!(
        challenge_id = challenge.id,
        action = "challenge_code_resent",
        "auth action"
    );
    Ok(())
}

/// Starts a password reset for an email.
///
/// # Anti-enumeration
///
/// The caller answers every submitted address identically, so
/// this function must not leak existence either: an address
/// with no account (or not even a valid address) returns
/// `Ok(None)` — no row, no mail, no error — while an address
/// with an account creates a challenge, sends the code and
/// returns `Ok(Some(id))` for the caller's session. A mail
/// failure is logged rather than surfaced, for the same reason:
/// an error shown only for real accounts would reveal them.
/// The resend button is the member's recovery.
///
/// # Privacy
///
/// The code is generated here, hashed before storage, and
/// passed only to the mail sender. It is never logged.
pub async fn start_password_reset(pool: &PgPool, email: &str) -> AuthResult<Option<i64>> {
    let email = email.trim().to_lowercase();
    if !validation::is_valid_email(&email) {
        return Ok(None);
    }

    // Housekeeping: abandoned resets past their grace day are
    // deleted here, the one place new resets begin.
    let _ = challenge_repository::delete_expired(pool, ChallengeTable::PasswordResets).await;

    let Some(user_id) = user_repository::id_for_email(pool, &email)
        .await
        .map_err(db_error)?
    else {
        return Ok(None);
    };

    let code = generate_code();
    let code_hash = hash_code(&code, &email);
    let expires_at = Utc::now() + Duration::minutes(CODE_LIFETIME_MINUTES);
    let reset_id = challenge_repository::create(
        pool,
        ChallengeTable::PasswordResets,
        user_id,
        &email,
        &code_hash,
        expires_at,
    )
    .await
    .map_err(db_error)?;

    let (name, kana, phone) = match user_repository::find_by_id(pool, user_id)
        .await
        .ok()
        .flatten()
    {
        Some(user) => mail_name_parts(&user),
        None => (String::new(), String::new(), String::new()),
    };
    if let Err(e) = mail::send_password_reset_code(pool, &email, &code, &name, &kana, &phone).await
    {
        tracing::error!(reset_id, "password reset mail failed: {e}");
    }

    // Only IDs and the action are logged: never the email or code.
    tracing::info!(
        user_id,
        reset_id,
        action = "password_reset_started",
        "auth action"
    );
    Ok(Some(reset_id))
}

/// Completes a password reset: the entered password must pass
/// the shared rule, then it replaces the stored hash, the
/// challenge is consumed, and every existing session of the
/// member is revoked — a reset usually means the account may be
/// compromised, so no old session may survive it.
pub async fn complete_password_reset(
    pool: &PgPool,
    challenge: &CodeChallenge,
    new_password: &str,
) -> AuthResult<()> {
    if !challenge.verified {
        return Err(refused(VERIFICATION_CODE_INCOMPLETE_MESSAGE));
    }
    if let Some(message) = password::password_rule_error(new_password) {
        return Err(refused(message));
    }
    let hash = password::hash_password_async(new_password.to_string()).await?;
    user_repository::update_password_hash(pool, challenge.user_id, &hash)
        .await
        .map_err(db_error)?;
    challenge_repository::delete_by_id(pool, ChallengeTable::PasswordResets, challenge.id)
        .await
        .map_err(db_error)?;
    revoke_sessions(pool, challenge.user_id, None).await;
    tracing::info!(
        user_id = challenge.user_id,
        action = "password_reset_completed",
        "auth action"
    );
    Ok(())
}

/// Starts an email change for a signed-in member: the
/// current password is re-confirmed, the new address must be
/// well-formed, different from the current one and not already
/// registered, and the code goes to the NEW address — the
/// account's email moves only when that code verifies.
pub async fn start_email_change(
    pool: &PgPool,
    user: &User,
    current_password: &str,
    new_email: &str,
) -> AuthResult<i64> {
    let stored = user_repository::password_hash_for_id(pool, user.id)
        .await
        .map_err(db_error)?
        .ok_or_else(|| refused("メールアドレスの変更に失敗しました。"))?;
    if !password::verify_password_async(current_password.to_string(), stored).await {
        return Err(refused(CURRENT_PASSWORD_MISMATCH_MESSAGE));
    }
    let new_email = new_email.trim().to_lowercase();
    if !validation::is_valid_email(&new_email) {
        return Err(refused(EMAIL_FORMAT_MESSAGE));
    }
    if new_email == user.email {
        return Err(refused("現在と同じメールアドレスです。"));
    }
    // The duplicate wording matches sign-up's refusal, minus
    // its login advice: this member is already signed in.
    if user_repository::exists_by_email(pool, &new_email)
        .await
        .map_err(db_error)?
    {
        return Err(refused(EMAIL_ALREADY_REGISTERED_MESSAGE));
    }

    // Housekeeping, as at every flow start.
    let _ = challenge_repository::delete_expired(pool, ChallengeTable::EmailChanges).await;

    let code = generate_code();
    let code_hash = hash_code(&code, &new_email);
    let expires_at = Utc::now() + Duration::minutes(CODE_LIFETIME_MINUTES);
    let change_id = challenge_repository::create(
        pool,
        ChallengeTable::EmailChanges,
        user.id,
        &new_email,
        &code_hash,
        expires_at,
    )
    .await
    .map_err(db_error)?;
    let (name, kana, phone) = mail_name_parts(user);
    mail::send_email_change_code(pool, &new_email, &code, &name, &kana, &phone)
        .await
        .map_err(|_| refused(MAIL_SEND_FAILED_MESSAGE))?;

    // Only IDs and the action are logged: never the addresses or code.
    tracing::info!(
        user_id = user.id,
        change_id,
        action = "email_change_started",
        "auth action"
    );
    Ok(change_id)
}

/// Completes an email change: the address is re-checked (it may
/// have been registered while the code was in flight), then the
/// account moves. Orders, coupons and reviews key on the user ID,
/// so nothing else moves with the address, and the member's
/// session — which also holds only the user ID — stays signed in.
pub async fn complete_email_change(pool: &PgPool, challenge: &CodeChallenge) -> AuthResult<()> {
    if !challenge.verified {
        return Err(refused(VERIFICATION_CODE_INCOMPLETE_MESSAGE));
    }
    if user_repository::exists_by_email(pool, &challenge.email)
        .await
        .map_err(db_error)?
    {
        return Err(refused(EMAIL_ALREADY_REGISTERED_MESSAGE));
    }
    user_repository::update_email(pool, challenge.user_id, &challenge.email)
        .await
        .map_err(db_error)?;
    challenge_repository::delete_by_id(pool, ChallengeTable::EmailChanges, challenge.id)
        .await
        .map_err(db_error)?;
    tracing::info!(
        user_id = challenge.user_id,
        action = "email_changed",
        "auth action"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DetailsForm, parse_birth_date, validate_details};

    fn valid_form() -> DetailsForm {
        DetailsForm {
            password: "abc12345".to_string(),
            family_name: "山田".to_string(),
            given_name: "太郎".to_string(),
            family_name_kana: "ヤマダ".to_string(),
            given_name_kana: "タロウ".to_string(),
            birth_date: String::new(),
            gender: String::new(),
            postal_code: "1500001".to_string(),
            prefecture: "東京都".to_string(),
            city: "渋谷区".to_string(),
            address_line: "道玄坂1-1-1".to_string(),
            building: String::new(),
            phone: "09012345678".to_string(),
            mail_magazine: false,
        }
    }

    #[test]
    fn valid_details_have_no_errors() {
        assert!(validate_details(&valid_form()).is_empty());
    }

    #[test]
    fn every_bad_field_is_reported_at_once() {
        let mut form = valid_form();
        form.password = "short".to_string();
        form.family_name_kana = "やまだ".to_string();
        form.postal_code = "123".to_string();
        form.phone = "123".to_string();
        let fields: Vec<_> = validate_details(&form).iter().map(|e| e.field).collect();
        assert!(fields.contains(&"password"));
        assert!(fields.contains(&"family_name_kana"));
        assert!(fields.contains(&"postal_code"));
        assert!(fields.contains(&"phone"));
    }

    #[test]
    fn password_needs_a_letter_and_a_digit() {
        let mut form = valid_form();
        form.password = "abcdefgh".to_string();
        assert!(
            validate_details(&form)
                .iter()
                .any(|e| e.field == "password")
        );
        form.password = "12345678".to_string();
        assert!(
            validate_details(&form)
                .iter()
                .any(|e| e.field == "password")
        );
    }

    #[test]
    fn birth_date_parsing() {
        assert!(parse_birth_date("").unwrap().is_none());
        assert!(parse_birth_date("1990-01-02").unwrap().is_some());
        assert!(parse_birth_date("1990/01/02").is_err());
    }
}
