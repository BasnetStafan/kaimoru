//! Notification mail templates, read side and renderer.
//!
//! A send resolves its mail's effective template — the
//! operator's stored override when one exists, otherwise
//! the code default from the catalog — through a short
//! in-process cache in the shop-settings shape, then
//! substitutes the send's context: the tag values and
//! generated blocks the sending flow computed from the
//! live record. Substitution is the whole engine: there
//! is no logic in a template, and a tag the context does
//! not fill renders empty.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::messages::{
    BODY_REQUIRED_MESSAGE, REFUND_METHOD_NOTE_MESSAGE, SUBJECT_REQUIRED_MESSAGE,
};
use crate::models::notification_template::{
    EffectiveTemplate, NotificationContext, NotificationMailDef, NotificationTemplate,
};
use crate::models::{Order, OrderLine, Payment, ShopSettings};
use crate::repositories::notification_template_repository;
use crate::validation::FieldError;

/// How long overrides may be served from memory before
/// the database is asked again.
const OVERRIDE_TTL: Duration = Duration::from_secs(60);

/// The cached overrides by mail key, and when they were
/// read. Process-local, like the settings cache.
static OVERRIDE_CACHE: Mutex<Option<(Instant, HashMap<String, NotificationTemplate>)>> =
    Mutex::new(None);

/// Returns the stored override for one mail, if any.
///
/// A failed read renders from the code default instead of
/// failing the send: a broken override read must never
/// stop a receipt or a verification code from going out.
/// A failed read is not cached.
pub async fn load_override(pool: &PgPool, mail_key: &str) -> Option<NotificationTemplate> {
    if let Ok(cache) = OVERRIDE_CACHE.lock()
        && let Some((read_at, rows)) = cache.as_ref()
        && read_at.elapsed() < OVERRIDE_TTL
    {
        return rows.get(mail_key).cloned();
    }
    let rows = match notification_template_repository::list_overrides(pool).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "notification template read failed; using code defaults");
            return None;
        }
    };
    let map: HashMap<String, NotificationTemplate> = rows
        .into_iter()
        .map(|row| (row.mail_key.clone(), row))
        .collect();
    let found = map.get(mail_key).cloned();
    if let Ok(mut cache) = OVERRIDE_CACHE.lock() {
        *cache = Some((Instant::now(), map));
    }
    found
}

/// Drops the cached overrides, so the next send re-reads
/// them. The admin save/reset calls this after committing.
pub fn invalidate_cache() {
    if let Ok(mut cache) = OVERRIDE_CACHE.lock() {
        *cache = None;
    }
}

/// Resolves one mail's effective template.
pub async fn effective_template(pool: &PgPool, def: &NotificationMailDef) -> EffectiveTemplate {
    match load_override(pool, def.key).await {
        Some(row) => EffectiveTemplate {
            subject: row.subject,
            body: row.body,
            customized: true,
        },
        None => def.default_template(),
    }
}

/// Renders one mail for sending: resolves the effective
/// template and substitutes the context into both parts.
pub async fn render_mail(
    pool: &PgPool,
    def: &NotificationMailDef,
    ctx: &NotificationContext,
) -> (String, String) {
    let template = effective_template(pool, def).await;
    (
        render_inline(&template.subject, ctx),
        render_text(&template.body, ctx),
    )
}

/// Substitutes the context into a body template.
///
/// Every tag is substituted in place; block values carry
/// their own newlines, so an empty block mid-body leaves
/// the blank line the template's structure implies. The
/// one exception is the body's LAST line: a line there
/// consisting of exactly one tag whose value is empty
/// (the shop URL line, say) drops out entirely, so a
/// conditional tail leaves no dangling blank line.
pub fn render_text(template: &str, ctx: &NotificationContext) -> String {
    let lines: Vec<&str> = template.split('\n').collect();
    // The last content line: the final split segment is
    // empty whenever the template ends with a newline.
    let last_content = if lines.last().is_some_and(|l| l.is_empty()) {
        lines.len().saturating_sub(2)
    } else {
        lines.len().saturating_sub(1)
    };
    let mut kept: Vec<String> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index == last_content
            && let Some(name) = sole_tag(line)
            && ctx.get(name).is_none_or(|v| v.is_empty())
        {
            continue;
        }
        kept.push(substitute(line, ctx));
    }
    kept.join("\n")
}

/// Substitutes the context into a single line (subjects):
/// no line dropping, tags are simply replaced.
pub fn render_inline(template: &str, ctx: &NotificationContext) -> String {
    substitute(template, ctx)
}

/// Returns the tag name when a line is exactly `{name}`.
fn sole_tag(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('{')?.strip_suffix('}')?;
    if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        Some(inner)
    } else {
        None
    }
}

/// Replaces every `{tag}` in `text` with its context
/// value; unknown or unfilled tags render empty (saved
/// templates are validated, so an unknown tag can only
/// come from a code default, which the golden tests pin).
fn substitute(text: &str, ctx: &NotificationContext) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    if let Some(value) = ctx.get(name) {
                        out.push_str(value);
                    }
                } else {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Scans a template for its `{tag}` names, in order.
/// A `{` that never closes yields one empty name, which
/// validation reports as a malformed tag.
fn scan_tags(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                names.push(after[..close].to_string());
                rest = &after[close + 1..];
            }
            None => {
                names.push(String::new());
                break;
            }
        }
    }
    names
}

/// Validates an operator's subject and body for one mail:
/// presence and length, every tag in the mail's closed
/// set, and the structural tags still present in the
/// body. Errors key to the form's fields.
pub fn validate_template(
    def: &NotificationMailDef,
    subject: &str,
    body: &str,
) -> Result<(String, String), Vec<FieldError>> {
    let subject = subject.trim().to_string();
    let body = body.trim_end().to_string();
    let mut errors: Vec<FieldError> = Vec::new();
    if subject.is_empty() {
        errors.push(FieldError::new("subject", SUBJECT_REQUIRED_MESSAGE));
    } else if subject.chars().count() > 200 {
        errors.push(FieldError::new(
            "subject",
            "件名は200文字以内で入力してください。",
        ));
    }
    if subject.contains('\n') {
        errors.push(FieldError::new("subject", "件名に改行は使えません。"));
    }
    for name in scan_tags(&subject) {
        if !def.allows(&name) {
            errors.push(FieldError::new(
                "subject",
                "このメールで使えないタグが含まれています。",
            ));
            break;
        }
    }
    if body.trim().is_empty() {
        errors.push(FieldError::new("body", BODY_REQUIRED_MESSAGE));
    } else if body.chars().count() > 20_000 {
        errors.push(FieldError::new(
            "body",
            "本文は20,000文字以内で入力してください。",
        ));
    }
    let body_tags = scan_tags(&body);
    for name in &body_tags {
        if !def.allows(name) {
            errors.push(FieldError::new(
                "body",
                "このメールで使えないタグが含まれています。使えるタグは下の一覧をご確認ください。",
            ));
            break;
        }
    }
    for required in def.required {
        if !body_tags.iter().any(|name| name == required) {
            errors.push(FieldError::new(
                "body",
                "自動生成ブロック（{...}）は削除できません。位置を変えることはできます。",
            ));
            break;
        }
    }
    // The required-tag message names the first missing tag.
    if let Some(err) = errors
        .iter_mut()
        .find(|e| e.field == "body" && e.message.contains("{...}"))
        && let Some(missing) = def
            .required
            .iter()
            .find(|r| !body_tags.iter().any(|name| name == **r))
    {
        err.message = format!(
            "自動生成ブロック（{{{missing}}}）は削除できません。位置を変えることはできます。"
        );
    }
    if errors.is_empty() {
        Ok((subject, body))
    } else {
        Err(errors)
    }
}

// ----------------------------------------------------------------
// Contexts: the values one send substitutes.
// ----------------------------------------------------------------

/// Starts a context with the shop identity tags.
fn shop_context(settings: &ShopSettings, shop_url: &str) -> NotificationContext {
    let mut ctx = NotificationContext::new();
    ctx.insert("shop_name", settings.shop_name.clone());
    ctx.insert(
        "signature",
        if settings.mail_signature.trim().is_empty() {
            settings.shop_name.clone()
        } else {
            settings.mail_signature.clone()
        },
    );
    ctx.insert("contact_email", settings.contact_email.clone());
    ctx.insert("shop_url", shop_url.to_string());
    ctx
}

/// Builds the context for a verification-code mail. The
/// name fields are empty when the flow has no account yet
/// (sign-up), which the editor notes on those tags.
pub fn code_context(
    settings: &ShopSettings,
    shop_url: &str,
    email: &str,
    name: &str,
    name_kana: &str,
    phone: &str,
    code: &str,
) -> NotificationContext {
    let mut ctx = shop_context(settings, shop_url);
    ctx.insert("customer_name", name.to_string());
    ctx.insert("customer_name_kana", name_kana.to_string());
    ctx.insert("customer_email", email.to_string());
    ctx.insert("customer_phone", phone.to_string());
    ctx.insert("code", code.to_string());
    ctx
}

/// Builds the context for the back-in-stock notification:
/// the shopper is an address, not an account, so only the
/// email recipient tag is filled and the product tags carry
/// the mail.
pub fn restock_context(
    settings: &ShopSettings,
    shop_url: &str,
    email: &str,
    product_name: &str,
    variant_label: &str,
    product_url: &str,
) -> NotificationContext {
    let mut ctx = shop_context(settings, shop_url);
    ctx.insert("customer_email", email.to_string());
    ctx.insert("product_name", product_name.to_string());
    ctx.insert("variant_label", variant_label.to_string());
    ctx.insert("product_url", product_url.to_string());
    ctx
}

/// Builds the low-stock alert context: the operator mail
/// that lists each low product with its remaining stock and
/// its admin edit link, one line per product.
pub fn low_stock_alert_context(
    settings: &ShopSettings,
    shop_url: &str,
    items: &[(i64, String, i64)],
) -> NotificationContext {
    let mut ctx = shop_context(settings, shop_url);
    let mut block = String::new();
    for (id, name, stock) in items {
        block.push_str(&format!(
            "・{name}（残り{stock}点） {shop_url}/admin/products/{id}/edit\n"
        ));
    }
    ctx.insert("products_block", block);
    ctx
}

/// Builds the context for the cart reminder: the
/// cart's lines as one block plus the cart page URL.
pub fn cart_reminder_context(
    settings: &ShopSettings,
    shop_url: &str,
    email: &str,
    lines: &[(String, i32)],
    unsubscribe_url: &str,
) -> NotificationContext {
    let mut ctx = shop_context(settings, shop_url);
    ctx.insert("customer_email", email.to_string());
    let mut block = String::new();
    for (name, qty) in lines {
        block.push_str(&format!("・{name} ×{qty}\n"));
    }
    ctx.insert("cart_block", block);
    ctx.insert("cart_url", format!("{shop_url}/cart"));
    ctx.insert("unsubscribe_url", unsubscribe_url.to_string());
    ctx
}

/// Builds the shared context for the order mails
/// (confirmation, payment, konbini, cancellation base).
pub fn order_context(
    settings: &ShopSettings,
    shop_url: &str,
    order: &Order,
    lines: &[OrderLine],
    breakdown: &[crate::models::tax::OrderTaxBreakdown],
    payment_method_label: &str,
) -> NotificationContext {
    let mut ctx = shop_context(settings, shop_url);
    // Recipient: the order snapshot's buyer, so guest
    // orders fill these too. The snapshot carries no kana.
    ctx.insert(
        "customer_name",
        format!("{} {}", order.family_name, order.given_name),
    );
    ctx.insert("customer_name_kana", String::new());
    ctx.insert("customer_email", order.email.clone());
    ctx.insert("customer_phone", order.phone.clone());
    // Order facts.
    ctx.insert("order_number", order.order_number());
    ctx.insert("order_date", order.placed_long());
    ctx.insert("order_status", order.status_label().to_string());
    ctx.insert(
        "item_count",
        lines.iter().map(|l| l.quantity).sum::<i32>().to_string(),
    );
    ctx.insert(
        "subtotal",
        crate::pricing::display_price(order.subtotal_yen),
    );
    ctx.insert(
        "discount",
        if order.discount_yen > 0 {
            crate::pricing::display_price(order.discount_yen)
        } else {
            String::new()
        },
    );
    ctx.insert(
        "shipping_fee",
        crate::pricing::display_price(order.shipping_yen),
    );
    ctx.insert(
        "cod_fee",
        if order.cod_fee_yen > 0 {
            crate::pricing::display_price(order.cod_fee_yen)
        } else {
            String::new()
        },
    );
    ctx.insert("total", crate::pricing::display_price(order.total_yen));
    ctx.insert("payment_method", payment_method_label.to_string());
    ctx.insert("coupon_code", order.coupon_code.clone().unwrap_or_default());
    // Delivery facts.
    ctx.insert(
        "delivery_name",
        format!("{} {}", order.family_name, order.given_name),
    );
    ctx.insert("delivery_postal", order.postal_code.clone());
    ctx.insert(
        "delivery_address",
        format!(
            "{}{}{}{}",
            order.prefecture,
            order.city,
            order.address_line,
            order.building.as_deref().unwrap_or_default()
        ),
    );
    ctx.insert("delivery_phone", order.phone.clone());
    ctx.insert(
        "delivery_date",
        order
            .delivery_date
            .map(crate::models::delivery::format_delivery_date_long)
            .unwrap_or_default(),
    );
    ctx.insert("delivery_slot", order.delivery_slot_label.clone());
    ctx.insert(
        "carrier",
        order
            .carrier
            .as_deref()
            .and_then(crate::models::carrier::carrier_for)
            .map(|c| c.label.to_string())
            .unwrap_or_default(),
    );
    ctx.insert(
        "tracking_number",
        order.tracking_number.clone().unwrap_or_default(),
    );
    ctx.insert(
        "tracking_url",
        match (&order.carrier, &order.tracking_number) {
            (Some(code), Some(number)) => {
                crate::models::carrier::tracking_url(code, number).unwrap_or_default()
            }
            _ => String::new(),
        },
    );
    // Generated blocks, in the order mails' layout.
    ctx.insert("items_block", items_block(lines, false));
    ctx.insert("tax_block", tax_block(breakdown));
    ctx.insert(
        "delivery_block",
        crate::models::delivery::delivery_mail_lines(
            order.delivery_date,
            &order.delivery_slot_label,
        ),
    );
    ctx.insert(
        "gift_block",
        crate::models::gift::gift_mail_lines(
            &order.noshi_code,
            &order.noshi_label,
            order.noshi_name.as_deref(),
            order.gift_message.as_deref(),
        ),
    );
    ctx.insert(
        "address_block",
        format!(
            "お届け先:{} {} 様\n〒{} {} {} {}\n",
            order.family_name,
            order.given_name,
            order.postal_code,
            order.prefecture,
            order.city,
            order.address_line
        ),
    );
    ctx
}

/// Adds the konbini payment block to an order context,
/// built from the payment row exactly as the built-in
/// mail has always built it.
pub fn add_konbini_block(ctx: &mut NotificationContext, payment: &Payment) {
    let mut extra = String::new();
    if let Some(store) = payment.store_label() {
        extra.push_str(&format!("お支払い店舗:{store}\n"));
    }
    if let Some(deadline) = payment.deadline_long() {
        extra.push_str(&format!("お支払い期限:{deadline}\n"));
    }
    if let Some(url) = &payment.instructions_url {
        extra.push_str(&format!("お支払い方法のご案内:{url}\n"));
    }
    extra.push_str("期限を過ぎるとご注文は自動でキャンセルされます。\n入金の確認が取れ次第、商品の発送準備を進めます。\n");
    ctx.insert("konbini_block", extra);
}

/// Builds the shipping notification's context: the order
/// context with the shipping layout's own blocks.
pub fn shipping_context(
    settings: &ShopSettings,
    shop_url: &str,
    order: &Order,
    lines: &[OrderLine],
    breakdown: &[crate::models::tax::OrderTaxBreakdown],
    shipped_date: Option<chrono::NaiveDate>,
) -> NotificationContext {
    let mut ctx = order_context(settings, shop_url, order, lines, breakdown, "");
    ctx.insert("items_block", items_block(lines, true));
    ctx.insert(
        "address_block",
        format!(
            "{} {} 様\n〒{}\n{}{}{}{}\n電話番号：{}\n",
            order.family_name,
            order.given_name,
            order.postal_code,
            order.prefecture,
            order.city,
            order.address_line,
            order.building.as_deref().unwrap_or_default(),
            order.phone
        ),
    );
    ctx.insert(
        "shipped_date",
        shipped_date
            .map(crate::models::delivery::format_delivery_date_long)
            .unwrap_or_default(),
    );
    ctx.insert(
        "shipped_date_block",
        shipped_date
            .map(|day| {
                format!(
                    "\n■ 発送日\n{}\n",
                    crate::models::delivery::format_delivery_date_long(day)
                )
            })
            .unwrap_or_default(),
    );
    ctx.insert(
        "cod_block",
        if order.cod_fee_yen > 0 {
            format!(
                "代金引換：お届け時に{}をご用意ください\n",
                crate::pricing::display_price(order.total_yen)
            )
        } else {
            String::new()
        },
    );
    ctx.insert(
        "delivery_request_block",
        order
            .delivery_date
            .map(|day| {
                format!(
                    "\n■ お届け希望\nお届け日：{}\n時間帯：{}\n",
                    crate::models::delivery::format_delivery_date_long(day),
                    order.delivery_slot_label
                )
            })
            .unwrap_or_default(),
    );
    ctx.insert("tracking_block", tracking_block(order));
    ctx.insert(
        "contact_block",
        if settings.contact_email.is_empty() {
            String::new()
        } else {
            format!("\nお問い合わせ：{}\n", settings.contact_email)
        },
    );
    ctx
}

/// The shipping mail's tracking section, or empty when
/// the operator entered no tracking number.
fn tracking_block(order: &Order) -> String {
    let Some(number) = &order.tracking_number else {
        return String::new();
    };
    let mut block = "\n■ 配送情報\n".to_string();
    if let Some(code) = &order.carrier
        && let Some(carrier) = crate::models::carrier::carrier_for(code)
    {
        block.push_str(&format!("配送会社：{}\n", carrier.label));
    }
    block.push_str(&format!("お問い合わせ伝票番号：{number}\n"));
    if let Some(code) = &order.carrier
        && let Some(url) = crate::models::carrier::tracking_url(code, number)
    {
        block.push_str(&format!("配送状況の確認はこちら：{url}\n"));
    }
    block
}

/// Builds the cancellation context: the order context
/// plus the one money paragraph the refund state selects
/// (the caller supplies the paragraph; its wording is
/// the mail module's legal copy, not a tag).
pub fn cancellation_context(
    settings: &ShopSettings,
    shop_url: &str,
    order: &Order,
    lines: &[OrderLine],
    breakdown: &[crate::models::tax::OrderTaxBreakdown],
    money_paragraph: &str,
) -> NotificationContext {
    let mut ctx = order_context(settings, shop_url, order, lines, breakdown, "");
    ctx.insert("money_block", money_paragraph.to_string());
    ctx
}

/// Builds the inquiry acknowledgement's context.
pub fn inquiry_context(
    settings: &ShopSettings,
    shop_url: &str,
    name: &str,
    email: &str,
    reference: &str,
    category_label: &str,
    body_text: &str,
) -> NotificationContext {
    let mut ctx = shop_context(settings, shop_url);
    ctx.insert("customer_name", name.to_string());
    ctx.insert("customer_name_kana", String::new());
    ctx.insert("customer_email", email.to_string());
    ctx.insert("customer_phone", String::new());
    ctx.insert("reference", reference.to_string());
    ctx.insert("category", category_label.to_string());
    ctx.insert("inquiry_body", body_text.to_string());
    ctx
}

/// Builds the withdrawal-completed context.
pub fn withdrawal_context(
    settings: &ShopSettings,
    shop_url: &str,
    name: &str,
    email: &str,
    date_label: &str,
) -> NotificationContext {
    let mut ctx = shop_context(settings, shop_url);
    ctx.insert("customer_name", name.to_string());
    ctx.insert("customer_name_kana", String::new());
    ctx.insert("customer_email", email.to_string());
    ctx.insert("customer_phone", String::new());
    ctx.insert("withdrawal_date", date_label.to_string());
    ctx
}

/// The order item lines: the order mails pack the
/// quantity against the ×, the shipping mail spaces it —
/// the layouts are each mail's own and never drift,
/// because both come from here.
fn items_block(lines: &[OrderLine], spaced: bool) -> String {
    let mut block = String::new();
    for line in lines {
        let name = if line.variant_label.is_empty() {
            line.product_name.clone()
        } else {
            format!("{}（{}）", line.product_name, line.variant_label)
        };
        if spaced {
            block.push_str(&format!(
                "{} × {} — {}\n",
                name,
                line.quantity,
                crate::pricing::display_price(line.line_total_yen())
            ));
        } else {
            block.push_str(&format!(
                "{} ×{} — {}\n",
                name,
                line.quantity,
                crate::pricing::display_price(line.line_total_yen())
            ));
        }
    }
    block
}

/// The per-class tax breakdown section, shared wording
/// across the order and shipping mails; empty when the
/// order carries no breakdown.
fn tax_block(breakdown: &[crate::models::tax::OrderTaxBreakdown]) -> String {
    if breakdown.is_empty() {
        return String::new();
    }
    let mut block = "\n税率ごとの内訳\n".to_string();
    for row in breakdown {
        block.push_str(&format!(
            "{}（{}%）対象:{}（うち消費税:{}）\n",
            row.tax_class_name,
            row.rate_percent,
            crate::pricing::display_price(row.taxable_base_yen),
            crate::pricing::display_price(row.tax_yen),
        ));
    }
    block
}

// ----------------------------------------------------------------
// Sample data for the editor's preview and test send.
// ----------------------------------------------------------------

/// Builds a fully populated sample context for one mail,
/// from fixed sample records (the same records the golden
/// tests pin), so the operator previews and test-sends
/// exactly the shape a real send produces. `sample_email`
/// fills the recipient tags: the test send goes to the
/// operator, and the preview shows their own address.
pub fn sample_context(
    def: &NotificationMailDef,
    settings: &ShopSettings,
    shop_url: &str,
    sample_email: &str,
) -> NotificationContext {
    let order = sample_order();
    let lines = sample_lines();
    let breakdown = sample_breakdown();
    let today = chrono::Utc::now()
        .with_timezone(&crate::models::jst::jst())
        .date_naive();
    match def.key {
        "verification_code" | "password_reset_code" | "email_change_code" | "withdrawal_code" => {
            code_context(
                settings,
                shop_url,
                sample_email,
                "山田 太郎",
                "ヤマダ タロウ",
                "090-0000-0000",
                "123456",
            )
        }
        "order_confirmation" => {
            order_context(settings, shop_url, &order, &lines, &breakdown, "代金引換")
        }
        "payment_confirmation" => order_context(
            settings,
            shop_url,
            &order,
            &lines,
            &breakdown,
            "クレジットカード",
        ),
        "konbini_instructions" => {
            let mut ctx = order_context(settings, shop_url, &order, &lines, &breakdown, "コンビニ");
            add_konbini_block(&mut ctx, &sample_payment());
            ctx
        }
        "shipping_notification" => {
            shipping_context(settings, shop_url, &order, &lines, &breakdown, Some(today))
        }
        "order_cancellation" => cancellation_context(
            settings,
            shop_url,
            &order,
            &lines,
            &breakdown,
            REFUND_METHOD_NOTE_MESSAGE,
        ),
        "inquiry_acknowledgement" => inquiry_context(
            settings,
            shop_url,
            "山田 太郎",
            sample_email,
            "INQ-000123",
            "商品について",
            "サンプルのお問い合わせ本文です。",
        ),
        "restock_notification" => restock_context(
            settings,
            shop_url,
            sample_email,
            "Rust Tシャツ",
            "サイズ: L",
            &format!("{shop_url}/products/1"),
        ),
        "cart_reminder" => cart_reminder_context(
            settings,
            shop_url,
            sample_email,
            &[("Rust Tシャツ".to_string(), 1)],
            &format!("{shop_url}/unsubscribe/sample"),
        ),
        "low_stock_alert" => {
            low_stock_alert_context(settings, shop_url, &[(1, "Rust Tシャツ".to_string(), 3)])
        }
        _ => withdrawal_context(
            settings,
            shop_url,
            "山田 太郎",
            sample_email,
            &today.format("%Y年%m月%d日").to_string(),
        ),
    }
}

/// The fixed sample order the previews and golden tests
/// share: two lines, one tax class, a delivery request,
/// a carrier and a tracking number.
fn sample_order() -> Order {
    use chrono::TimeZone;
    Order {
        id: 2193,
        user_id: Some(8),
        email: "test@example.com".to_string(),
        family_name: "山田".to_string(),
        given_name: "太郎".to_string(),
        postal_code: "1000001".to_string(),
        prefecture: "東京都".to_string(),
        city: "千代田区千代田".to_string(),
        address_line: "1-2-3".to_string(),
        building: Some("サンプルマンション101".to_string()),
        phone: "090-0000-0000".to_string(),
        subtotal_yen: 4660,
        points_used_yen: 0,
        points_earned: 0,
        discount_yen: 500,
        shipping_yen: 550,
        shipping_discount_yen: 0,
        cod_fee_yen: 330,
        total_yen: 5040,
        coupon_code: Some("WELCOME500".to_string()),
        status: "received".to_string(),
        placed_via: "storefront".to_string(),
        delivery_date: Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()),
        delivery_slot: "14-16".to_string(),
        delivery_slot_label: "14時〜16時".to_string(),
        carrier: Some("yamato".to_string()),
        tracking_number: Some("123456789012".to_string()),
        shipped_mail_sent_at: None,
        created_at: chrono::Utc.with_ymd_and_hms(2026, 11, 7, 5, 11, 0).unwrap(),
        noshi_code: "none".to_string(),
        noshi_label: "なし".to_string(),
        noshi_name: None,
        gift_message: None,
    }
}

/// The sample order's lines.
fn sample_lines() -> Vec<OrderLine> {
    vec![
        OrderLine {
            product_id: 1,
            product_name: "Rust Tシャツ".to_string(),
            variant_label: "サイズ: L".to_string(),
            unit_price_yen: 2980,
            quantity: 1,
            tax_class_code: Some("standard".to_string()),
            rate_percent: Some(10),
        },
        OrderLine {
            product_id: 5,
            product_name: "SQLx キーホルダー".to_string(),
            variant_label: String::new(),
            unit_price_yen: 880,
            quantity: 2,
            tax_class_code: Some("standard".to_string()),
            rate_percent: Some(10),
        },
    ]
}

/// The sample order's tax breakdown.
fn sample_breakdown() -> Vec<crate::models::tax::OrderTaxBreakdown> {
    vec![crate::models::tax::OrderTaxBreakdown {
        tax_class_code: "standard".to_string(),
        tax_class_name: "標準税率".to_string(),
        rate_percent: 10,
        taxable_base_yen: 5040,
        tax_yen: 458,
    }]
}

/// The sample konbini payment.
fn sample_payment() -> Payment {
    use chrono::TimeZone;
    Payment {
        id: 1,
        order_id: 2193,
        provider: "komoju".to_string(),
        method: "konbini".to_string(),
        status: "authorized".to_string(),
        amount_yen: 5040,
        amount_refunded_yen: 0,
        currency: "JPY".to_string(),
        komoju_session_id: None,
        komoju_payment_id: None,
        stripe_session_id: None,
        stripe_payment_id: None,
        external_order_num: None,
        payment_deadline: Some(
            chrono::Utc
                .with_ymd_and_hms(2026, 11, 10, 14, 59, 0)
                .unwrap(),
        ),
        instructions_url: Some("https://komoju.example/instructions".to_string()),
        payment_details: Some(serde_json::json!({"store": "seven-eleven"})),
        flag_note: None,
        captured_at: None,
        confirmation_mail_sent_at: None,
        compensation_pending: false,
        created_at: chrono::Utc.with_ymd_and_hms(2026, 11, 7, 5, 11, 0).unwrap(),
        updated_at: chrono::Utc.with_ymd_and_hms(2026, 11, 7, 5, 11, 0).unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::notification_template::find_def;

    /// Settings matching the golden capture: the shipping
    /// suite's shop name, a contact address, no signature.
    fn golden_settings() -> ShopSettings {
        ShopSettings {
            shop_name: "テスト店舗SM".to_string(),
            contact_email: "support@example.com".to_string(),
            ..ShopSettings::fallback()
        }
    }

    /// Renders one mail's code default against a context
    /// and returns the body.
    fn render_default(key: &str, ctx: &NotificationContext) -> String {
        let def = find_def(key).expect("catalog key");
        render_text(def.default_body, ctx)
    }

    #[test]
    fn order_confirmation_default_matches_legacy() {
        let settings = golden_settings();
        let ctx = order_context(
            &settings,
            "https://example.com/",
            &sample_order(),
            &sample_lines(),
            &sample_breakdown(),
            "代金引換",
        );
        assert_eq!(
            render_default("order_confirmation", &ctx),
            r####"AGENTIC STORE をご利用いただきありがとうございます。
ご注文を受け付けました。

注文番号:AS-20261107-2193


Rust Tシャツ（サイズ: L） ×1 — ￥2,980
SQLx キーホルダー ×2 — ￥1,760

税率ごとの内訳
標準税率（10%）対象:￥5,040（うち消費税:￥458）

合計:￥5,040（お届け時に現金でお支払いください）

お届け希望日：2026年11月10日（火）
お届け時間帯：14時〜16時
お届け先:山田 太郎 様
〒1000001 東京都 千代田区千代田 1-2-3
"####
        );
    }

    #[test]
    fn payment_confirmation_default_matches_legacy() {
        let settings = golden_settings();
        let ctx = order_context(
            &settings,
            "https://example.com/",
            &sample_order(),
            &sample_lines(),
            &sample_breakdown(),
            "クレジットカード",
        );
        assert_eq!(
            render_default("payment_confirmation", &ctx),
            r####"AGENTIC STORE をご利用いただきありがとうございます。
お支払いが完了しました。

注文番号:AS-20261107-2193


Rust Tシャツ（サイズ: L） ×1 — ￥2,980
SQLx キーホルダー ×2 — ￥1,760

税率ごとの内訳
標準税率（10%）対象:￥5,040（うち消費税:￥458）

合計:￥5,040（お支払い済み）

商品の発送準備を進めています。
お届け希望日：2026年11月10日（火）
お届け時間帯：14時〜16時
お届け先:山田 太郎 様
〒1000001 東京都 千代田区千代田 1-2-3
"####
        );
    }

    #[test]
    fn konbini_default_matches_legacy() {
        let settings = golden_settings();
        let mut ctx = order_context(
            &settings,
            "https://example.com/",
            &sample_order(),
            &sample_lines(),
            &sample_breakdown(),
            "コンビニ",
        );
        add_konbini_block(&mut ctx, &sample_payment());
        assert_eq!(
            render_default("konbini_instructions", &ctx),
            r####"AGENTIC STORE をご利用いただきありがとうございます。
コンビニ支払いのご案内です。

注文番号:AS-20261107-2193


Rust Tシャツ（サイズ: L） ×1 — ￥2,980
SQLx キーホルダー ×2 — ￥1,760

税率ごとの内訳
標準税率（10%）対象:￥5,040（うち消費税:￥458）

合計:￥5,040（コンビニでお支払いください）

お支払い店舗:セブン-イレブン
お支払い期限:2026年11月10日 23:59
お支払い方法のご案内:https://komoju.example/instructions
期限を過ぎるとご注文は自動でキャンセルされます。
入金の確認が取れ次第、商品の発送準備を進めます。
お届け希望日：2026年11月10日（火）
お届け時間帯：14時〜16時
お届け先:山田 太郎 様
〒1000001 東京都 千代田区千代田 1-2-3
"####
        );
    }

    #[test]
    fn shipping_default_matches_legacy_full() {
        let settings = golden_settings();
        let ctx = shipping_context(
            &settings,
            "https://example.com/",
            &sample_order(),
            &sample_lines(),
            &sample_breakdown(),
            Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()),
        );
        assert_eq!(
            render_default("shipping_notification", &ctx),
            r####"山田 太郎 様

いつもテスト店舗SMをご利用いただき、誠にありがとうございます。
ご注文いただいた商品を発送いたしましたので、お知らせいたします。

■ ご注文番号
AS-20261107-2193

■ 発送日
2026年11月8日（日）

■ ご注文内容
Rust Tシャツ（サイズ: L） × 1 — ￥2,980
SQLx キーホルダー × 2 — ￥1,760
合計：￥5,040（税込）

税率ごとの内訳
標準税率（10%）対象:￥5,040（うち消費税:￥458）

■ お届け先
山田 太郎 様
〒1000001
東京都千代田区千代田1-2-3サンプルマンション101
電話番号：090-0000-0000
代金引換：お届け時に￥5,040をご用意ください

■ お届け希望
お届け日：2026年11月10日（火）
時間帯：14時〜16時

■ 配送情報
配送会社：ヤマト運輸
お問い合わせ伝票番号：123456789012
配送状況の確認はこちら：https://jizen.kuronekoyamato.co.jp/jizen/servlet/crjz.b.NQ0010?id=123456789012

商品がお手元に届くまで、今しばらくお待ちくださいますようお願いいたします。
お届け内容に相違がある場合や、商品の破損などがございましたら、
お手数ですが下記までご連絡ください。

お問い合わせ：support@example.com
──────────────
テスト店舗SM
https://example.com/
"####
        );
    }

    #[test]
    fn shipping_default_matches_legacy_plain() {
        let mut settings = golden_settings();
        settings.contact_email = String::new();
        let mut order = sample_order();
        order.cod_fee_yen = 0;
        order.carrier = None;
        order.tracking_number = None;
        order.delivery_date = None;
        order.building = None;
        let ctx = shipping_context(&settings, "", &order, &sample_lines(), &[], None);
        assert_eq!(
            render_default("shipping_notification", &ctx),
            r####"山田 太郎 様

いつもテスト店舗SMをご利用いただき、誠にありがとうございます。
ご注文いただいた商品を発送いたしましたので、お知らせいたします。

■ ご注文番号
AS-20261107-2193

■ ご注文内容
Rust Tシャツ（サイズ: L） × 1 — ￥2,980
SQLx キーホルダー × 2 — ￥1,760
合計：￥5,040（税込）

■ お届け先
山田 太郎 様
〒1000001
東京都千代田区千代田1-2-3
電話番号：090-0000-0000

商品がお手元に届くまで、今しばらくお待ちくださいますようお願いいたします。
お届け内容に相違がある場合や、商品の破損などがございましたら、
お手数ですが下記までご連絡ください。
──────────────
テスト店舗SM
"####
        );
    }

    #[test]
    fn code_default_matches_legacy() {
        let settings = golden_settings();
        let ctx = code_context(&settings, "", "test@example.com", "", "", "", "123456");
        assert_eq!(
            render_default("verification_code", &ctx),
            "AGENTIC STORE をご利用いただきありがとうございます。\n\n会員登録の認証コードは次のとおりです。\n\n認証コード:123456\n\nこのコードの有効期限は10分間です。\n心当たりがない場合は、このメールを破棄してください。\n"
        );
    }

    #[test]
    fn sole_tag_line_with_empty_value_drops() {
        let mut ctx = NotificationContext::new();
        ctx.insert("shop_name", "店".to_string());
        ctx.insert("shop_url", String::new());
        assert_eq!(render_text("{shop_name}\n{shop_url}\n", &ctx), "店\n");
    }

    #[test]
    fn validation_rejects_unknown_and_missing_required() {
        let def = find_def("verification_code").unwrap();
        let err = validate_template(def, "件名", "コードは {code} と {nope}").unwrap_err();
        assert!(err.iter().any(|e| e.field == "body"));
        let err = validate_template(def, "件名", "コードなし").unwrap_err();
        assert!(err.iter().any(|e| e.field == "body"));
        assert!(validate_template(def, "件名", "認証コード:{code}").is_ok());
    }
}
