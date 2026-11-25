//! Marketing mail core: rendering and sending.
//!
//! Rendering substitutes the member variables and product
//! blocks into a campaign's stored bodies and appends the
//! consent footer (shop name + the recipient's own
//! unsubscribe link). Sending never happens inside an HTTP
//! request: the worker loop claims queued campaign jobs and
//! walks their deliveries, re-checking consent at send time.
//! Transactional mail does not pass through here at all.

use std::collections::HashMap;

use rand::RngCore;
use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::mail;
use crate::models::mail::{EmailCampaign, EmailDelivery};
use crate::repositories::{mail_repository, product_repository, user_repository};
use crate::services::{mail_subscription_service, shop_settings_service};

/// Send attempts a delivery gets before it is marked failed.
const MAX_ATTEMPTS: i32 = 3;
/// Deliveries processed per worker batch.
const BATCH_SIZE: i64 = 50;
/// Minutes a claimed delivery may sit in `sending` before
/// a later pass treats its worker as crashed and reclaims
/// it. A pass sends its whole batch in seconds; ten minutes
/// is far beyond any live claim.
const STALE_CLAIM_MINUTES: i64 = 10;

/// One fully rendered mail: the three parts the transport
/// sends for one recipient.
pub struct RenderedMail {
    /// Rendered subject line.
    pub subject: String,
    /// Rendered HTML body, footer included.
    pub html: String,
    /// Rendered plain-text body, footer included.
    pub text: String,
}

/// The store's public base URL for links inside mail, when
/// `PUBLIC_BASE_URL` is configured; empty in local
/// development, where links stay root-relative paths.
fn public_base() -> String {
    CONFIG
        .public_base_url()
        .map(|base| base.trim_end_matches('/').to_string())
        .unwrap_or_default()
}

/// Builds one member's unsubscribe URL from their token.
/// The token is a credential: it appears only in mail
/// addressed to that member.
pub fn unsubscribe_url(token: &str) -> String {
    format!("{}/unsubscribe/{}", public_base(), token)
}

/// Generates one tracking token: 32 random bytes as hex
/// (the unsubscribe-token shape). The token is a random
/// id: it encodes nothing about the recipient and is the
/// lookup key for its tracking row.
fn new_track_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The distinct http(s) href targets in a rendered HTML
/// body that click tracking may wrap. The unsubscribe
/// link is never wrapped (it must stay a direct consent
/// action, and counting it as engagement would be wrong),
/// and neither are tracking URLs themselves.
fn trackable_hrefs(html: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for quote in ['"', '\''] {
        let needle = format!("href={quote}");
        let mut rest = html;
        while let Some(pos) = rest.find(&needle) {
            let after = &rest[pos + needle.len()..];
            let Some(end) = after.find(quote) else { break };
            let url = &after[..end];
            if (url.starts_with("http://") || url.starts_with("https://"))
                && !url.contains("/unsubscribe/")
                && !url.contains("/mail/track/")
                && !urls.iter().any(|u| u == url)
            {
                urls.push(url.to_string());
            }
            rest = &after[end + 1..];
        }
    }
    urls
}

/// Rewrites each tracked href in the HTML body to its
/// redirect URL. Only the exact `href="..."` / `href='...'`
/// attribute values change; the URLs stay visible in the
/// plain-text part, which is never rewritten.
fn rewrite_hrefs(html: &str, pairs: &[(String, String)], base: &str) -> String {
    let mut out = html.to_string();
    for (url, token) in pairs {
        let tracked = format!("{base}/mail/track/click/{token}");
        out = out.replace(&format!("href=\"{url}\""), &format!("href=\"{tracked}\""));
        out = out.replace(&format!("href='{url}'"), &format!("href='{tracked}'"));
    }
    out
}

/// Applies C2 open/click tracking to one rendered
/// campaign mail: ensures the delivery's open token,
/// stores one link row per trackable href and rewrites
/// those hrefs through the redirect endpoint, then
/// appends the 1x1 open pixel. Tokens are created once
/// per delivery and reused across send retries, so a
/// retried delivery never mails dead links.
async fn apply_tracking(
    pool: &PgPool,
    delivery: &EmailDelivery,
    rendered: RenderedMail,
) -> Result<RenderedMail, AppError> {
    let base = public_base();
    let open_token = match &delivery.open_token {
        Some(token) => token.clone(),
        None => {
            let token = new_track_token();
            mail_repository::set_open_token(pool, delivery.id, &token).await?;
            token
        }
    };
    let mut pairs: Vec<(String, String)> = Vec::new();
    for url in trackable_hrefs(&rendered.html) {
        let token = match mail_repository::track_link_token(pool, delivery.id, &url).await? {
            Some(token) => token,
            None => {
                let token = new_track_token();
                mail_repository::insert_track_link(pool, delivery.id, &url, &token).await?;
                token
            }
        };
        pairs.push((url, token));
    }
    let mut html = rewrite_hrefs(&rendered.html, &pairs, &base);
    html.push_str(&format!(
        "\n<img src=\"{base}/mail/track/open/{open_token}\" width=\"1\" height=\"1\" alt=\"\" style=\"display:none\">"
    ));
    Ok(RenderedMail {
        subject: rendered.subject,
        html,
        text: rendered.text,
    })
}

/// Escapes a value interpolated into the HTML body. Product
/// and member names are data, not markup: the campaign body
/// was sanitised at save, and interpolated values must not
/// be able to smuggle tags past that.
fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Renders one campaign for one recipient: replaces
/// `{{product:<id>}}` blocks with the live product's facts,
/// substitutes `{name}` and `{unsubscribe_url}`, and
/// appends the consent footer to both bodies. The blocks
/// expand first, so a member name carrying a block token
/// stays literal data; the name is escaped in the HTML
/// body, raw in the text body and the subject.
///
/// A product block whose product is missing or not public
/// renders as nothing and logs a warning — a mail must
/// never carry a hand-typed product copy that drifted from
/// the catalogue.
pub async fn render(
    pool: &PgPool,
    campaign: &EmailCampaign,
    member_name: &str,
    unsubscribe_url: &str,
) -> Result<RenderedMail, AppError> {
    let shop = shop_settings_service::load(pool).await;
    let subject = substitute(&campaign.subject, member_name, unsubscribe_url, false);
    let mut html = expand_product_blocks(pool, &campaign.body_html, true).await;
    let mut text = expand_product_blocks(pool, &campaign.body_text, false).await;
    html = substitute(&html, member_name, unsubscribe_url, true);
    text = substitute(&text, member_name, unsubscribe_url, false);
    html.push_str(&format!(
"<hr>\n<p>{}<br>\nメールマガジンの配信停止は<a href=\"{}\">こちら</a>からお手続きください。</p>",
escape_html(&shop.shop_name),
escape_html(unsubscribe_url),
));
    text.push_str(&format!(
        "\n\n──────────────\n{}\nメールマガジンの配信停止はこちらからお手続きください。\n{}\n",
        shop.shop_name, unsubscribe_url,
    ));
    Ok(RenderedMail {
        subject,
        html,
        text,
    })
}

/// Substitutes the two member variables into one body.
/// With `html` set, the member name is escaped first: it is
/// data interpolated into markup the campaign author was
/// allowed to write, and must not smuggle in tags of its
/// own. The text body and the subject keep the raw name.
fn substitute(body: &str, member_name: &str, unsubscribe_url: &str, html: bool) -> String {
    let name = if html {
        escape_html(member_name)
    } else {
        member_name.to_string()
    };
    body.replace("{name}", &name)
        .replace("{unsubscribe_url}", unsubscribe_url)
}

/// The distinct product ids a body's blocks name, in
/// first-appearance order. An unparseable id is skipped
/// here; the render pass warns about it in place.
fn product_block_ids(body: &str) -> Vec<i64> {
    let mut ids: Vec<i64> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("{{product:") {
        let after = &rest[start + "{{product:".len()..];
        match after.find("}}") {
            Some(end) => {
                if let Ok(id) = after[..end].trim().parse::<i64>()
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
                rest = &after[end + 2..];
            }
            // An unterminated block is literal text; a
            // later block still counts.
            None => rest = after,
        }
    }
    ids
}

/// Replaces every `{{product:<id>}}` block in one body.
/// The named products load in ONE bulk query from the live
/// catalogue first: the price shown is the product's
/// current effective (sale-aware) price, so a mail can
/// never quote a stale hand-typed number — and the
/// per-block, per-recipient reads this replaces no longer
/// multiply with the campaign's audience.
async fn expand_product_blocks(pool: &PgPool, body: &str, html: bool) -> String {
    let ids = product_block_ids(body);
    let (products, lookup_failed) = match product_repository::catalog_products(pool, &ids).await {
        Ok(rows) => (
            rows.into_iter()
                .map(|p| (p.id, p))
                .collect::<HashMap<i64, crate::models::Product>>(),
            false,
        ),
        Err(e) => {
            tracing::warn!("mail product block lookup failed: {e}");
            (HashMap::new(), true)
        }
    };
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("{{product:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "{{product:".len()..];
        match after.find("}}") {
            Some(end) => {
                let raw_id = after[..end].trim();
                match raw_id.parse::<i64>() {
                    Ok(id) => match products.get(&id) {
                        Some(product) => out.push_str(&product_block(product, html)),
                        None if !lookup_failed => {
                            tracing::warn!(
                                product_id = id,
                                "mail product block skipped: not public"
                            );
                        }
                        None => {}
                    },
                    Err(_) => {
                        tracing::warn!(raw_id, "mail product block with an unparseable id");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                // An unterminated block is literal text.
                out.push_str("{{product:");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Renders one product block from its loaded product.
fn product_block(product: &crate::models::Product, html: bool) -> String {
    let price = crate::pricing::display_price(product.effective_price_yen());
    let url = format!("{}/products/{}", public_base(), product.id);
    if html {
        let name = escape_html(&product.name);
        let image = if product.image_path.is_empty() {
            String::new()
        } else {
            format!(
                "<a href=\"{}\"><img src=\"{}{}\" alt=\"{}\" style=\"max-width:240px;\"></a><br>\n",
                escape_html(&url),
                public_base(),
                escape_html(&product.image_path),
                name,
            )
        };
        format!(
            "<div style=\"border:1px solid #dddddd;padding:12px;margin:12px 0;\">\n\
{image}<a href=\"{}\">{}</a><br>\n{}（税込）\n</div>",
            escape_html(&url),
            name,
            price,
        )
    } else {
        format!("\n{}\n{}（税込）\n{}\n", product.name, price, url)
    }
}

/// Releases one delivery's claim after a pre-send error,
/// logging a failed release: the row then waits for the
/// stale-claim reclaim instead of the next pass.
async fn release_claim(pool: &PgPool, delivery_id: i64) {
    if let Err(e) = mail_repository::release_delivery(pool, delivery_id).await {
        tracing::error!(delivery_id, "release delivery failed: {e}");
    }
}

/// Sends one campaign as a test to the operating admin:
/// rendered with the admin's display name and a placeholder
/// unsubscribe token, delivered immediately outside the job
/// queue — no job or delivery rows are written. Returns a
/// user-safe message on failure.
pub async fn send_test(
    pool: &PgPool,
    campaign: &EmailCampaign,
    admin_name: &str,
    admin_email: &str,
) -> Result<(), String> {
    let rendered = render(pool, campaign, admin_name, &unsubscribe_url("test"))
        .await
        .map_err(|e| format!("テスト送信の準備に失敗しました：{e}"))?;
    mail::send_marketing(
        admin_email,
        &format!("【テスト】{}", rendered.subject),
        &rendered.html,
        &rendered.text,
    )
    .await
    .map_err(|e| format!("テスト送信に失敗しました：{e}"))
}

/// Starts the mail delivery worker beside the other
/// background loops, with the same watch-channel shutdown
/// contract: the pass in flight finishes, then the loop
/// stops.
pub fn start_worker(
    pool: PgPool,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let interval_secs = CONFIG.mail_worker_interval_secs.max(5);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
            }
            if let Err(e) = run_once(&pool).await {
                tracing::error!("mail worker pass failed: {e}");
            }
        }
        tracing::info!("mail delivery worker stopped");
    });
    tracing::info!(interval_secs, "mail delivery worker started");
    handle
}

/// Runs one worker pass: returns stale claims to the
/// queue, continues the running job or claims the oldest
/// due one, then sends a bounded batch of deliveries it
/// claimed for itself. When a job's queue empties, the job
/// and its campaign are marked complete.
async fn run_once(pool: &PgPool) -> Result<(), AppError> {
    // A worker that died mid-batch leaves its claims in
    // `sending`; once stale, they belong to the queue again.
    match mail_repository::reclaim_stale_claims(pool, STALE_CLAIM_MINUTES).await {
        Ok(reclaimed) if reclaimed > 0 => {
            tracing::warn!(reclaimed, "mail worker reclaimed stale delivery claims");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!("mail stale-claim reclaim failed: {e}");
        }
    }
    let job = match mail_repository::find_running_job(pool).await? {
        Some(job) => job,
        None => {
            let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
            let claimed = mail_repository::claim_next_job(&mut tx).await?;
            let Some(job) = claimed else {
                return Ok(());
            };
            mail_repository::set_campaign_status_tx(&mut tx, job.campaign_id, "sending").await?;
            tx.commit().await.map_err(|e| AppError::db("commit", e))?;
            job
        }
    };
    let Some(campaign) = mail_repository::find_campaign(pool, job.campaign_id).await? else {
        tracing::error!(job_id = job.id, "mail job without a campaign; finishing it");
        mail_repository::finish_job(pool, job.id).await?;
        return Ok(());
    };
    let deliveries = mail_repository::claim_deliveries(pool, job.id, BATCH_SIZE).await?;
    // Deliveries in one claimed batch are independent -
    // each re-checks consent for its own recipient - so
    // the batch sends with bounded concurrency,
    // paced to the configured send-rate ceiling:
    // after each wave, the pass sleeps out the rest of the
    // wave's rate window, keeping the sustained rate
    // under the provider quota.
    let concurrency = CONFIG.mail_send_concurrency.max(1) as usize;
    let rate = CONFIG.mail_send_rate_per_second;
    for wave in deliveries.chunks(concurrency) {
        let wave_started = std::time::Instant::now();
        let mut set = tokio::task::JoinSet::new();
        for delivery in wave {
            let pool = pool.clone();
            let campaign = campaign.clone();
            let delivery = delivery.clone();
            set.spawn(async move {
                process_delivery(&pool, &campaign, &delivery).await;
            });
        }
        while let Some(joined) = set.join_next().await {
            if let Err(e) = joined {
                tracing::error!("mail delivery task failed: {e}");
            }
        }
        if rate > 0 {
            let window = std::time::Duration::from_secs_f64(wave.len() as f64 / f64::from(rate));
            let elapsed = wave_started.elapsed();
            if elapsed < window {
                tokio::time::sleep(window - elapsed).await;
            }
        }
    }
    if mail_repository::count_queued_deliveries(pool, job.id).await? == 0 {
        mail_repository::finish_job(pool, job.id).await?;
        mail_repository::mark_campaign_sent(pool, campaign.id).await?;
        tracing::info!(
            job_id = job.id,
            campaign_id = campaign.id,
            action = "mail_campaign_sent",
            "mail campaign sent"
        );
    }
    Ok(())
}

/// Processes one claimed delivery: consent is re-checked
/// at send time (an unsubscribe after enqueue always
/// wins), then the mail is rendered for this recipient and
/// sent. Failures record the error and retry on later
/// passes until the attempt ceiling marks them failed. A
/// step that errors before any send attempt releases the
/// claim instead of recording an outcome, so a later pass
/// retries the delivery as if unclaimed.
async fn process_delivery(pool: &PgPool, campaign: &EmailCampaign, delivery: &EmailDelivery) {
    let may_send = match mail_subscription_service::may_send(pool, delivery.user_id).await {
        Ok(v) => v,
        Err(e) => {
            // A failed re-check is not an opt-out: treating
            // it as one would silently drop a subscribed
            // member's mail on a database hiccup.
            tracing::error!(
                delivery_id = delivery.id,
                "mail consent re-check failed: {e}"
            );
            release_claim(pool, delivery.id).await;
            return;
        }
    };
    if !may_send {
        if let Err(e) = mail_repository::mark_delivery_skipped(pool, delivery.id).await {
            tracing::error!(delivery_id = delivery.id, "mark skipped failed: {e}");
        }
        return;
    }
    let user = match user_repository::find_by_id(pool, delivery.user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            if let Err(e) = mail_repository::mark_delivery_skipped(pool, delivery.id).await {
                tracing::error!(delivery_id = delivery.id, "mark skipped failed: {e}");
            }
            return;
        }
        Err(e) => {
            // A transient read failure is not "the member is
            // gone": release the claim so a later pass retries
            // instead of terminally skipping the delivery.
            tracing::error!(
                delivery_id = delivery.id,
                "delivery member read failed: {e}"
            );
            release_claim(pool, delivery.id).await;
            return;
        }
    };
    let subscription = match mail_repository::find_subscription(pool, delivery.user_id).await {
        Ok(Some(subscription)) => subscription,
        Ok(None) => {
            if let Err(e) = mail_repository::mark_delivery_skipped(pool, delivery.id).await {
                tracing::error!(delivery_id = delivery.id, "mark skipped failed: {e}");
            }
            return;
        }
        Err(e) => {
            tracing::error!(
                delivery_id = delivery.id,
                "delivery subscription read failed: {e}"
            );
            release_claim(pool, delivery.id).await;
            return;
        }
    };
    let member_name = format!("{} {}", user.family_name, user.given_name);
    let rendered = match render(
        pool,
        campaign,
        &member_name,
        &unsubscribe_url(&subscription.unsubscribe_token),
    )
    .await
    {
        Ok(rendered) => rendered,
        Err(e) => {
            tracing::error!(delivery_id = delivery.id, "mail render failed: {e}");
            release_claim(pool, delivery.id).await;
            return;
        }
    };
    // Tracking is part of the mail itself: a failure to
    // store tokens releases the claim like a render
    // failure, so a campaign mail never goes out
    // untracked (C2 completion).
    let rendered = match apply_tracking(pool, delivery, rendered).await {
        Ok(rendered) => rendered,
        Err(e) => {
            tracing::error!(delivery_id = delivery.id, "mail tracking failed: {e}");
            release_claim(pool, delivery.id).await;
            return;
        }
    };
    let attempts = match mail_repository::record_attempt(pool, delivery.id).await {
        Ok(attempts) => attempts,
        Err(e) => {
            tracing::error!(delivery_id = delivery.id, "record attempt failed: {e}");
            release_claim(pool, delivery.id).await;
            return;
        }
    };
    // The transport error is boxed and not Send, so it is
    // flattened to text before any further await.
    let send_result = mail::send_marketing(
        &delivery.email,
        &rendered.subject,
        &rendered.html,
        &rendered.text,
    )
    .await
    .map_err(|e| e.to_string());
    match send_result {
        Ok(()) => {
            if let Err(e) = mail_repository::mark_delivery_sent(pool, delivery.id).await {
                tracing::error!(delivery_id = delivery.id, "mark sent failed: {e}");
            }
        }
        Err(message) => {
            let final_failure = attempts >= MAX_ATTEMPTS;
            tracing::warn!(
                delivery_id = delivery.id,
                attempts,
                final_failure,
                "mail send failed: {message}"
            );
            if let Err(e2) =
                mail_repository::mark_delivery_failure(pool, delivery.id, &message, final_failure)
                    .await
            {
                tracing::error!(delivery_id = delivery.id, "mark failure failed: {e2}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_replaces_both_variables() {
        let out = substitute(
            "{name} 様、停止は {unsubscribe_url} から",
            "山田 太郎",
            "https://example.jp/unsubscribe/abc",
            false,
        );
        assert_eq!(
            out,
            "山田 太郎 様、停止は https://example.jp/unsubscribe/abc から"
        );
    }

    #[test]
    fn substitute_escapes_the_name_only_for_html() {
        let name = "山田 <script>alert(1)</script>";
        let html = substitute("{name} 様", name, "https://example.jp/u", true);
        assert_eq!(html, "山田 &lt;script&gt;alert(1)&lt;/script&gt; 様");
        let text = substitute("{name} 様", name, "https://example.jp/u", false);
        assert_eq!(text, "山田 <script>alert(1)</script> 様");
    }

    #[tokio::test]
    async fn member_name_cannot_expand_a_product_block() {
        // The render order — blocks first, name second —
        // means a name carrying a block token stays literal.
        // The lazy pool never connects: the body below has no
        // block of its own, so expansion is a no-op.
        let pool = sqlx::PgPool::connect_lazy("postgres://invalid:invalid@localhost:1/none")
            .expect("lazy pool");
        let expanded = expand_product_blocks(&pool, "{name} 様へのお知らせ", true).await;
        let out = substitute(&expanded, "{{product:2}}", "https://example.jp/u", true);
        assert_eq!(out, "{{product:2}} 様へのお知らせ");
    }

    #[test]
    fn trackable_hrefs_skips_unsubscribe_and_dedupes() {
        let html = "<a href=\"https://shop.example.jp/products/1\">a</a>\
                    <a href=\"https://shop.example.jp/products/1\">b</a>\
                    <a href=\"https://shop.example.jp/unsubscribe/abc\">stop</a>\
                    <a href='/products/2'>relative</a>";
        assert_eq!(
            trackable_hrefs(html),
            vec!["https://shop.example.jp/products/1".to_string()]
        );
    }

    #[test]
    fn rewrite_hrefs_wraps_only_the_listed_urls() {
        let html = "<a href=\"https://shop.example.jp/products/1\">a</a>\
                    <a href=\"https://shop.example.jp/unsubscribe/abc\">stop</a>";
        let out = rewrite_hrefs(
            html,
            &[(
                "https://shop.example.jp/products/1".to_string(),
                "tok123".to_string(),
            )],
            "https://shop.example.jp",
        );
        assert!(out.contains("href=\"https://shop.example.jp/mail/track/click/tok123\""));
        assert!(out.contains("href=\"https://shop.example.jp/unsubscribe/abc\""));
    }

    #[test]
    fn escape_html_neutralises_markup_characters() {
        assert_eq!(
            escape_html("<a href=\"x\">&"),
            "&lt;a href=&quot;x&quot;&gt;&amp;"
        );
    }

    #[tokio::test]
    async fn unterminated_product_block_stays_literal() {
        // A lazy pool never connects: neither case below can
        // reach a query (unterminated opener, unparseable id).
        let pool = sqlx::PgPool::connect_lazy("postgres://invalid:invalid@localhost:1/none")
            .expect("lazy pool");
        let out = expand_product_blocks(&pool, "前 {{product:2 後", false).await;
        assert_eq!(out, "前 {{product:2 後");
        let out = expand_product_blocks(&pool, "a{{product:xyz}}b", false).await;
        assert_eq!(out, "ab");
    }
}
