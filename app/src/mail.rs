//! Outgoing mail for Kaimoru.
//!
//! Transactional bodies are templates now: each sender
//! builds the context for its mail (the tag values and
//! generated blocks from the live record) and the
//! notification template service renders the operator's
//! override or the code default. Verification codes are
//! sent through here and nowhere else.

use lettre::{
    AsyncTransport, Message, Tokio1Executor,
    message::header::ContentType,
    message::{MultiPart, SinglePart},
    transport::smtp::{
        AsyncSmtpTransport,
        authentication::Credentials,
        client::{Tls, TlsParameters},
    },
};
use sqlx::PgPool;

use crate::messages::REFUND_METHOD_NOTE_MESSAGE;
use crate::models::notification_template::{NotificationContext, find_def};
use crate::services::notification_template_service as templates;
use crate::services::shop_settings_service;

/// Returns the shop's public base URL from the
/// environment (the `{shop_url}` tag), or empty when the
/// deployment sets none.
fn shop_url() -> String {
    std::env::var("PUBLIC_BASE_URL").unwrap_or_default()
}

/// Renders one notification mail: resolves the catalog
/// entry by the sender's key and renders its effective
/// template against the context.
async fn compose(pool: &PgPool, key: &str, ctx: &NotificationContext) -> (String, String) {
    let def = find_def(key).expect("sender key is a catalog key");
    templates::render_mail(pool, def, ctx).await
}

/// Sends the sign-up verification code email.
///
/// The code exists only in this call and in the email
/// itself; it is never logged. The sign-up flow has no
/// account yet, so the caller passes empty name fields.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message.
pub async fn send_verification_code(
    pool: &PgPool,
    to_email: &str,
    code: &str,
    name: &str,
    name_kana: &str,
    phone: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::code_context(
        &settings,
        &shop_url(),
        to_email,
        name,
        name_kana,
        phone,
        code,
    );
    let (subject, body) = compose(pool, "verification_code", &ctx).await;
    send_plain(to_email, &subject, body).await
}

/// Sends the password reset code email.
///
/// The code exists only in this call and in the email
/// itself; it is never logged.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message.
pub async fn send_password_reset_code(
    pool: &PgPool,
    to_email: &str,
    code: &str,
    name: &str,
    name_kana: &str,
    phone: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::code_context(
        &settings,
        &shop_url(),
        to_email,
        name,
        name_kana,
        phone,
        code,
    );
    let (subject, body) = compose(pool, "password_reset_code", &ctx).await;
    send_plain(to_email, &subject, body).await
}

/// Sends the email change code email, to the NEW address:
/// proving the member controls it before the account's
/// login ID moves. The context names the account holder;
/// the recipient tags point at the new address.
///
/// The code exists only in this call and in the email
/// itself; it is never logged.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message.
pub async fn send_email_change_code(
    pool: &PgPool,
    to_email: &str,
    code: &str,
    name: &str,
    name_kana: &str,
    phone: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::code_context(
        &settings,
        &shop_url(),
        to_email,
        name,
        name_kana,
        phone,
        code,
    );
    let (subject, body) = compose(pool, "email_change_code", &ctx).await;
    send_plain(to_email, &subject, body).await
}

/// Sends the withdrawal proof code email, to the
/// account's own address: a passwordless account proves
/// ownership by reading its mailbox instead of entering
/// a password.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message.
pub async fn send_withdrawal_code(
    pool: &PgPool,
    to_email: &str,
    code: &str,
    name: &str,
    name_kana: &str,
    phone: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::code_context(
        &settings,
        &shop_url(),
        to_email,
        name,
        name_kana,
        phone,
        code,
    );
    let (subject, body) = compose(pool, "withdrawal_code", &ctx).await;
    send_plain(to_email, &subject, body).await
}

/// Builds and sends one plain-text mail; the senders
/// above differ only in subject and body.
///
/// # Errors
///
/// Returns an error when the message cannot be built or
/// the mail server cannot be reached or rejects it.
async fn send_plain(
    to_email: &str,
    subject: &str,
    body: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let email = Message::builder()
        .from(from_address()?)
        .to(parse_recipient(to_email)?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body)?;
    let mailer = mailer()?;
    mailer.send(email).await?;
    Ok(())
}

/// Sends one already-rendered plain-text mail (the
/// notification template editor's test send).
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message.
pub async fn send_rendered_plain(
    to_email: &str,
    subject: &str,
    body: String,
) -> Result<(), Box<dyn std::error::Error>> {
    send_plain(to_email, subject, body).await
}

/// Sends one back-in-stock notification: the mail the
/// restock sweep sends when a subscribed combination's
/// stock returns. One-shot by design — the subscription
/// completes with this send, so there is no unsubscribe
/// link to carry.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message; the sweep returns the
/// subscription to pending and retries on a later pass.
pub async fn send_restock_notification(
    pool: &PgPool,
    to_email: &str,
    product_id: i64,
    product_name: &str,
    variant_label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let product_url = format!("{}/products/{}", shop_url(), product_id);
    let ctx = templates::restock_context(
        &settings,
        &shop_url(),
        to_email,
        product_name,
        variant_label,
        &product_url,
    );
    let (subject, body) = compose(pool, "restock_notification", &ctx).await;
    send_plain(to_email, &subject, body).await?;
    Ok(())
}

/// Sends one cart-abandonment reminder with the cart's
/// current lines. Plain text through the notification
/// template system, like every other mail.
pub async fn send_cart_reminder(
    pool: &PgPool,
    to_email: &str,
    lines: &[(String, i32)],
    unsubscribe_token: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let unsubscribe_url = unsubscribe_token
        .map(|token| format!("{}/unsubscribe/{}", shop_url(), token))
        .unwrap_or_default();
    let ctx =
        templates::cart_reminder_context(&settings, &shop_url(), to_email, lines, &unsubscribe_url);
    let (subject, body) = compose(pool, "cart_reminder", &ctx).await;
    send_plain(to_email, &subject, body).await?;
    Ok(())
}

/// Sends one low-stock alert mail to the shop's operator
/// address: the summary the alert worker sends when public
/// products sit at or below their thresholds.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message; the worker records nothing in
/// that case, so the next pass retries.
pub async fn send_low_stock_alert(
    pool: &PgPool,
    to_email: &str,
    items: &[(i64, String, i64)],
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::low_stock_alert_context(&settings, &shop_url(), items);
    let (subject, body) = compose(pool, "low_stock_alert", &ctx).await;
    send_plain(to_email, &subject, body).await?;
    Ok(())
}

/// Sends the order confirmation email for a placed order
/// (cash on delivery: the only door that confirms at
/// placement; KOMOJU orders confirm on payment).
///
/// The recipient and order contents exist only in this
/// call and in the email; callers log only the order ID
/// when sending fails.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message. A failed confirmation never
/// fails the order.
pub async fn send_order_confirmation(
    pool: &PgPool,
    order: &crate::models::Order,
    lines: &[crate::models::OrderLine],
    breakdown: &[crate::models::tax::OrderTaxBreakdown],
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::order_context(&settings, &shop_url(), order, lines, breakdown, "代金引換");
    let (subject, body) = compose(pool, "order_confirmation", &ctx).await;
    send_plain(&order.email, &subject, body).await?;
    Ok(())
}

/// Sends the payment confirmation for a settled KOMOJU
/// order: the same order summary as the COD confirmation,
/// with the money marked paid instead of due at the door.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message. A failed mail never fails the
/// payment.
pub async fn send_payment_confirmation(
    pool: &PgPool,
    order: &crate::models::Order,
    lines: &[crate::models::OrderLine],
    breakdown: &[crate::models::tax::OrderTaxBreakdown],
    payment_method_label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::order_context(
        &settings,
        &shop_url(),
        order,
        lines,
        breakdown,
        payment_method_label,
    );
    let (subject, body) = compose(pool, "payment_confirmation", &ctx).await;
    send_plain(&order.email, &subject, body).await?;
    Ok(())
}

/// Sends the convenience-store payment instructions for
/// a konbini order: the deadline and the hosted
/// instructions page the customer pays from. Sent once,
/// when the instructions are first recorded.
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message. A failed mail never fails the
/// order; the same details stay on the customer's order
/// page.
pub async fn send_konbini_instructions(
    pool: &PgPool,
    order: &crate::models::Order,
    lines: &[crate::models::OrderLine],
    breakdown: &[crate::models::tax::OrderTaxBreakdown],
    payment: &crate::models::Payment,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let mut ctx =
        templates::order_context(&settings, &shop_url(), order, lines, breakdown, "コンビニ");
    templates::add_konbini_block(&mut ctx, payment);
    let (subject, body) = compose(pool, "konbini_instructions", &ctx).await;
    send_plain(&order.email, &subject, body).await?;
    Ok(())
}

/// Builds the SMTP transport from the environment.
///
/// Two shapes, chosen by whether credentials are configured:
///
/// - No `MAIL_USER`: local development — plaintext to Mailpit.
/// - `MAIL_USER` + `MAIL_PASSWORD` set: production (Amazon SES).
///   Credentials are never sent without TLS: port 465 gets
///   implicit TLS, any other port must upgrade via STARTTLS.
///   Startup validation has already refused a half-configured
///   pair; this errors the same way as a second line of defence.
fn mailer() -> Result<AsyncSmtpTransport<Tokio1Executor>, Box<dyn std::error::Error>> {
    let host = std::env::var("MAIL_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port: u16 = std::env::var("MAIL_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1025);
    let builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host.clone()).port(port);

    let user = std::env::var("MAIL_USER")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let password = std::env::var("MAIL_PASSWORD")
        .ok()
        .filter(|v| !v.trim().is_empty());
    match (user, password) {
        (Some(user), Some(password)) => {
            // Credentials only ever travel inside TLS: 465 is
            // implicit TLS; other ports must refuse a server that
            // will not upgrade.
            let parameters = TlsParameters::new(host)?;
            let tls = if port == 465 {
                Tls::Wrapper(parameters)
            } else {
                Tls::Required(parameters)
            };
            Ok(builder
                .credentials(Credentials::new(user, password))
                .tls(tls)
                .build())
        }
        (None, None) => Ok(builder.build()),
        _ => Err(
            "MAIL_USER and MAIL_PASSWORD must be set together (see README / .env.example)".into(),
        ),
    }
}

/// Returns the From mailbox from `MAIL_FROM`; a bad value
/// is an error, never a panic.
fn from_address() -> Result<lettre::message::Mailbox, Box<dyn std::error::Error>> {
    let from =
        std::env::var("MAIL_FROM").unwrap_or_else(|_| "no-reply@agentic-store.local".to_string());
    Ok(from.parse()?)
}

/// Parses a recipient address the caller already validated.
fn parse_recipient(to_email: &str) -> Result<lettre::message::Mailbox, Box<dyn std::error::Error>> {
    Ok(to_email.parse()?)
}

/// Sends the shipping notification for a shipped order:
/// the second mail of the order lifecycle, after the
/// confirmation. Built from the order's snapshots, like
/// every order mail, so a later catalogue or profile edit
/// never rewrites what the customer was told. The
/// conditional blocks follow the data: the delivery
/// request only when the order carries one, the tracking
/// block only when a number was entered at ship time, the
/// shipped-date block only when the caller knows it (a
/// resend of an older order omits it rather than inventing
/// a date).
///
/// # Errors
///
/// Returns an error when the mail server cannot be reached
/// or rejects the message. A failed mail never fails the
/// shipment; the caller leaves the order's mail stamp
/// unset so the failure stays visible in the back office.
pub async fn send_shipping_notification(
    pool: &PgPool,
    order: &crate::models::Order,
    lines: &[crate::models::OrderLine],
    breakdown: &[crate::models::tax::OrderTaxBreakdown],
    shipped_date: Option<chrono::NaiveDate>,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::shipping_context(
        &settings,
        &shop_url(),
        order,
        lines,
        breakdown,
        shipped_date,
    );
    let (subject, body) = compose(pool, "shipping_notification", &ctx).await;
    send_plain(&order.email, &subject, body).await?;
    Ok(())
}

/// What a member cancellation means for the member's
/// money (Order Cancellation spec section 5.8): the mail
/// carries exactly one paragraph matching this state,
/// and never claims a refund that has not happened.
#[derive(Clone, Copy)]
pub enum CancellationMoney {
    /// Nothing was taken; nothing comes back.
    NoneTaken,
    /// The captured money has been refunded.
    Refunded,
    /// The refund is being processed (the automatic
    /// attempt did not land; the shop will complete it).
    Processing,
    /// Captured konbini money: refunded by hand after
    /// the shop contacts the member.
    KonbiniManual,
}

/// Returns the cancellation mail's money paragraph for
/// one refund state: the legal copy the template's
/// `{money_block}` inserts.
pub fn cancellation_money_paragraph(money: CancellationMoney) -> &'static str {
    match money {
        CancellationMoney::NoneTaken => "お支払いは発生していないため、返金はありません。",
        CancellationMoney::Refunded => REFUND_METHOD_NOTE_MESSAGE,
        CancellationMoney::Processing => "返金処理中です。完了までしばらくお待ちください。",
        CancellationMoney::KonbiniManual => {
            "コンビニ決済はオンラインで自動返金できません。\
返金方法（ご返金先口座の確認）について、店舗からご連絡します。"
        }
    }
}

/// Sends the order cancellation mail (Order Cancellation
/// spec section 5.8). Spawned off the request path by
/// the caller; a failure is logged there and never
/// rolls back the cancellation.
///
/// # Errors
///
/// Returns an error when the mail server cannot be
/// reached or rejects the message.
pub async fn send_order_cancellation(
    pool: &PgPool,
    order: &crate::models::Order,
    lines: &[crate::models::OrderLine],
    breakdown: &[crate::models::tax::OrderTaxBreakdown],
    money: CancellationMoney,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::cancellation_context(
        &settings,
        &shop_url(),
        order,
        lines,
        breakdown,
        cancellation_money_paragraph(money),
    );
    let (subject, body) = compose(pool, "order_cancellation", &ctx).await;
    send_plain(&order.email, &subject, body).await?;
    Ok(())
}

/// Sends the inquiry acknowledgement:
/// the reference number, the category and the customer's
/// own body quoted back, so the customer keeps a record
/// of what they sent.
///
/// # Errors
///
/// Returns an error when the mail server cannot be
/// reached or rejects the message. A failed
/// acknowledgement never fails the submission.
pub async fn send_inquiry_acknowledgement(
    pool: &PgPool,
    to_email: &str,
    reference: &str,
    category_label: &str,
    body_text: &str,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let ctx = templates::inquiry_context(
        &settings,
        &shop_url(),
        name,
        to_email,
        reference,
        category_label,
        body_text,
    );
    let (subject, body) = compose(pool, "inquiry_acknowledgement", &ctx).await;
    send_plain(to_email, &subject, body).await
}

/// Sends one inquiry reply exactly as the admin wrote
/// it.
///
/// # Errors
///
/// Returns an error when the mail server cannot be
/// reached or rejects the message; the caller records
/// the reply only when this succeeds.
pub async fn send_inquiry_reply(
    to_email: &str,
    subject: &str,
    body: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    send_plain(to_email, subject, body.to_string()).await
}

/// Sends the withdrawal completion mail (Account
/// Deletion spec D8): the member's only durable proof
/// of completion, sent after the withdrawal commits,
/// to the address captured before anonymisation. It
/// carries no order or account data beyond the
/// completion itself.
///
/// # Errors
///
/// Returns an error when the mail server cannot be
/// reached or rejects the message; the caller logs it
/// (the withdrawal stands either way).
pub async fn send_withdrawal_completed(
    pool: &PgPool,
    to_email: &str,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings = shop_settings_service::load(pool).await;
    let today = chrono::Utc::now()
        .with_timezone(&crate::models::jst::jst())
        .format("%Y年%m月%d日")
        .to_string();
    let ctx = templates::withdrawal_context(&settings, &shop_url(), name, to_email, &today);
    let (subject, body) = compose(pool, "withdrawal_completed", &ctx).await;
    send_plain(to_email, &subject, body).await?;
    Ok(())
}

/// Sends one marketing (mail magazine) message with HTML
/// and plain-text alternatives. It shares
/// the transport with the transactional senders — the
/// systems stay separate at the call site: only the
/// campaign service calls this, behind the consent
/// checks, and transactional mail never flows through
/// here.
///
/// # Errors
///
/// Returns an error when the message cannot be built or
/// the mail server cannot be reached or rejects it.
pub async fn send_marketing(
    to_email: &str,
    subject: &str,
    html: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let body = MultiPart::alternative()
        .singlepart(SinglePart::plain(text.to_string()))
        .singlepart(SinglePart::html(html.to_string()));
    let email = Message::builder()
        .from(from_address()?)
        .to(parse_recipient(to_email)?)
        .subject(subject)
        .multipart(body)?;
    let mailer = mailer()?;
    mailer.send(email).await?;
    Ok(())
}
