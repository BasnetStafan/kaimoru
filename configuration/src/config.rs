//! Typed application configuration, loaded once from `config.toml`.
//!
//! Fails fast when the file is missing or invalid. Secrets are not
//! part of this config: they come from environment variables.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

/// Returns the default log directory used when config.toml omits it.
fn default_log_dir() -> String {
    "logs".to_string()
}

/// Returns the default bind host: local development only.
///
/// The AWS deployment overrides it with `0.0.0.0` so the ALB can
/// reach the container.
fn default_bind_host() -> String {
    "127.0.0.1".to_string()
}

/// Returns the default database SSL mode: local PostgreSQL has no
/// TLS; the AWS deployment sets `require` for RDS.
fn default_db_ssl_mode() -> String {
    "disable".to_string()
}

/// Returns the default runtime database user: the least-
/// privilege role the server serves traffic as, while
/// `user` stays the owner that runs migrations.
fn default_db_app_user() -> String {
    "agentic_app".to_string()
}

/// Returns the default mail worker interval in seconds:
/// thirty seconds between campaign delivery passes.
fn default_mail_worker_interval_secs() -> u64 {
    30
}

/// Returns the default low-stock alert interval in seconds:
/// one pass an hour.
fn default_cart_reminder_delay_hours() -> i64 {
    24
}

fn default_cart_reminder_interval_secs() -> u64 {
    900
}

fn default_low_stock_alert_interval_secs() -> u64 {
    3600
}

/// Returns the default guest-cart retention in days: a guest
/// cart untouched for a month is treated as abandoned.
fn default_guest_cart_retention_days() -> i64 {
    30
}

/// Returns the default backup directory: the backup script's
/// established output location beside the project.
fn default_backup_dir() -> String {
    "./backups".to_string()
}

/// Returns the default mail send concurrency: how many
/// campaign deliveries one worker pass sends in parallel.
/// Bounded so a campaign cannot monopolise the pool or the
/// provider connection budget.
fn default_mail_send_concurrency() -> u32 {
    5
}

/// Returns the default mail send rate ceiling, in sends per
/// second: the pacing a provider quota (Amazon SES) imposes,
/// applied across one worker pass. Zero disables pacing.
fn default_mail_send_rate_per_second() -> u32 {
    10
}

/// Returns the default log retention in days: rotated daily
/// log files older than this are pruned by the server. Zero
/// keeps files forever.
fn default_log_retention_days() -> u32 {
    30
}

fn default_image_thumb_max_px() -> u32 {
    480
}

fn default_image_medium_max_px() -> u32 {
    1024
}

fn default_image_jpeg_quality() -> u8 {
    82
}

/// Returns the default payment sweep interval in seconds: ten
/// minutes between reconciliation passes over open payments.
fn default_payment_sweep_interval_secs() -> u64 {
    600
}

/// Returns the default hosted payment gateway: KOMOJU, the
/// rail this store has always run.
fn default_hosted_gateway() -> String {
    "komoju".to_string()
}

/// Returns the default admin list page size: twenty rows fill a
/// management screen without paging for everyday volumes.
fn default_admin_per_page() -> i64 {
    20
}

/// Returns the default per-member address-book cap: five saved
/// addresses cover home, work, family and a couple of gift
/// destinations without turning the picker into a scroll.
fn default_max_addresses() -> i64 {
    5
}

/// Returns the default storefront search page size: twenty
/// results, matching the admin lists.
fn default_search_per_page() -> i64 {
    20
}

/// Returns the default agent-API catalog page size: twenty,
/// matching the storefront search page size.
fn default_api_per_page() -> i64 {
    20
}

fn default_csv_max_rows() -> i64 {
    5000
}

fn default_csv_max_file_bytes() -> i64 {
    10_485_760
}

/// Returns the default product-review page size: ten, so the
/// review section stays a compact block on the product page.
fn default_reviews_per_page() -> i64 {
    10
}

/// Returns the default member order-history page size: ten, not
/// the search page's twenty — the history is scanned from the
/// account summary on a personal page, and ten rows keep that
/// page compact.
fn default_history_per_page() -> i64 {
    10
}

/// Returns the default number of orders in the mypage hub's
/// recent-orders block: five — a glance at what just happened,
/// with the full history one click away.
fn default_recent_orders_count() -> i64 {
    5
}

/// Returns the default number of products in one top page
/// section row: twenty (one config value decides how
/// many products every top page product section displays).
fn default_home_section_product_count() -> i64 {
    20
}

/// Returns the default number of categories the top
/// page's category nav shows: six — more than that and the nav links to the
/// category index instead of listing them all.
fn default_home_category_nav_count() -> i64 {
    6
}

/// Which half of the application this process runs (for running more than one process
/// against one database): everything, HTTP only, or
/// background tasks only. Background tasks are written to
/// be safe under several processes regardless (atomic
/// claims, advisory locks); the role exists so a deployment
/// can scale the halves independently, not to make them
/// correct.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppRole {
    /// HTTP and background tasks: the single-server setup.
    #[default]
    All,
    /// HTTP only: no background task starts in this process.
    Web,
    /// Background tasks only: no HTTP listener is bound.
    Worker,
}

impl AppRole {
    /// Whether this process serves HTTP.
    pub fn serves_http(&self) -> bool {
        matches!(self, Self::All | Self::Web)
    }

    /// Whether this process runs the background tasks.
    pub fn runs_tasks(&self) -> bool {
        matches!(self, Self::All | Self::Worker)
    }
}

/// Top-level application settings from `config.toml`; database
/// settings live in [`Database`].
///
/// Unknown keys are rejected: a misspelled key must fail the
/// load, not be silently ignored while the `#[serde(default)]`
/// value quietly takes its place.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Which half of the application this process runs; see
    /// [`AppRole`]. Absent means `all` — one process doing
    /// everything, as before the split existed.
    #[serde(default)]
    pub app_role: AppRole,
    /// How yen amounts are displayed, either "￥" or "円".
    pub yen_symbol: String,
    /// Port the HTTP server listens on.
    pub port: u16,
    /// Database connection settings, without any password.
    pub database: Database,
    /// Directory for log files, relative to the workspace root when not absolute.
    #[serde(default = "default_log_dir")]
    pub log_dir: String,
    /// Days of rotated log files to keep in `log_dir`
    ///; older files are deleted by the
    /// server's retention pass. Zero keeps everything.
    #[serde(default = "default_log_retention_days")]
    pub log_retention_days: u32,
    /// Host/interface the HTTP server binds to.
    #[serde(default = "default_bind_host")]
    pub bind_host: String,
    /// Whether the session cookie carries the Secure flag: false for
    /// local HTTP, true behind TLS at the ALB.
    #[serde(default)]
    pub cookie_secure: bool,
    /// Database SSL mode: "disable" locally, "require" on RDS.
    #[serde(default = "default_db_ssl_mode")]
    pub db_ssl_mode: String,
    /// Whether the KOMOJU payment methods are offered at checkout.
    /// The secrets themselves stay in environment variables; this
    /// flag only says the store wants the methods. When the flag
    /// is on but a secret is missing, the app still starts and
    /// serves cash on delivery only: a payment misconfiguration
    /// must never take the whole store down.
    #[serde(default)]
    pub komoju_enabled: bool,

    /// Master switch for Stripe payments (the second hosted
    /// gateway). Same fail-closed shape as `komoju_enabled`:
    /// the flag only says the store wants the rail; the
    /// gateway exists only while every secret resolves.
    #[serde(default)]
    pub stripe_enabled: bool,

    /// Master switch for SNS (social) login. Even when true, a
    /// provider is offered only while its credentials resolve
    /// from the environment - the same fail-closed shape as
    /// KOMOJU payments.
    #[serde(default)]
    pub sns_login_enabled: bool,
    /// Master switch for the MCP (Model Context Protocol)
    /// server mounted at `/mcp` for external AI clients. Off by
    /// default: the surface does not exist unless a deployment
    /// turns it on. It serves the public catalog only — the
    /// same data the storefront and the agent API already
    /// expose without a login.
    #[serde(default)]
    pub mcp_enabled: bool,
    /// Seconds between payment reconciliation sweep passes.
    #[serde(default = "default_payment_sweep_interval_secs")]
    pub payment_sweep_interval_secs: u64,
    /// Which hosted payment gateway checkout offers: "komoju"
    /// or "stripe". One rail is active at a time; payments
    /// already opened on the other rail keep settling through
    /// their own gateway regardless of this choice.
    #[serde(default = "default_hosted_gateway")]
    pub hosted_gateway: String,
    /// Longest edge, in pixels, of the thumbnail derivative
    /// generated beside an uploaded product image:
    /// the storefront's cards and the detail gallery strip
    /// serve it instead of the original. Zero skips the
    /// derivative.
    #[serde(default = "default_image_thumb_max_px")]
    pub image_thumb_max_px: u32,
    /// Longest edge, in pixels, of the medium derivative
    /// generated beside an uploaded product image:
    /// the detail page's main image serves it, and the
    /// original remains the lightbox view. Zero skips the
    /// derivative.
    #[serde(default = "default_image_medium_max_px")]
    pub image_medium_max_px: u32,
    /// JPEG quality (1-100) for derivatives of JPEG uploads
    ///: a JPEG original's derivatives stay JPEG,
    /// where a quality near 80 is visually transparent at
    /// card and detail sizes and far smaller than a lossless
    /// copy. PNG and WebP originals get lossless WebP
    /// derivatives instead, so transparency survives.
    #[serde(default = "default_image_jpeg_quality")]
    pub image_jpeg_quality: u8,
    /// How often the background mail worker checks for due
    /// campaign jobs, in seconds.
    #[serde(default = "default_mail_worker_interval_secs")]
    pub mail_worker_interval_secs: u64,
    /// How often the low-stock alert pass runs, in seconds:
    /// it mails the operator at most once per product per
    /// day, so the interval only bounds detection latency.
    #[serde(default = "default_low_stock_alert_interval_secs")]
    pub low_stock_alert_interval_secs: u64,
    /// Hours a member cart must sit untouched before its
    /// one reminder mail.
    #[serde(default = "default_cart_reminder_delay_hours")]
    pub cart_reminder_delay_hours: i64,
    /// How often the cart reminder pass runs, in seconds.
    #[serde(default = "default_cart_reminder_interval_secs")]
    pub cart_reminder_interval_secs: u64,
    /// Days a guest cart may sit untouched before the
    /// cleanup sweep deletes it. Member carts are
    /// never swept. Zero or negative disables the sweep.
    #[serde(default = "default_guest_cart_retention_days")]
    pub guest_cart_retention_days: i64,
    /// Directory the ops status panel reads the newest DB
    /// backup from (the backup script's output directory).
    /// A directory the app cannot read shows as
    /// 確認できません on the panel, never an error.
    #[serde(default = "default_backup_dir")]
    pub backup_dir: String,
    /// Campaign deliveries one worker pass sends in
    /// parallel.
    #[serde(default = "default_mail_send_concurrency")]
    pub mail_send_concurrency: u32,
    /// Ceiling on campaign sends per second across a
    /// worker pass, paced to the SES
    /// quota at deployment. Zero disables pacing.
    #[serde(default = "default_mail_send_rate_per_second")]
    pub mail_send_rate_per_second: u32,
    /// Rows per page on the admin list screens.
    #[serde(default = "default_admin_per_page")]
    pub admin_per_page: i64,
    /// Results per page on the storefront search page.
    #[serde(default = "default_search_per_page")]
    pub search_per_page: i64,
    /// Products per page on the agent API's catalog list.
    #[serde(default = "default_api_per_page")]
    pub api_per_page: i64,
    /// Data-row ceiling for one admin CSV import file.
    #[serde(default = "default_csv_max_rows")]
    pub csv_max_rows: i64,
    /// Byte ceiling for one admin CSV import upload.
    #[serde(default = "default_csv_max_file_bytes")]
    pub csv_max_file_bytes: i64,
    /// Reviews per page on the product page.
    #[serde(default = "default_reviews_per_page")]
    pub reviews_per_page: i64,
    /// Orders per page in the member order history.
    #[serde(default = "default_history_per_page")]
    pub history_per_page: i64,
    /// Orders shown in the mypage hub's recent-orders block.
    #[serde(default = "default_recent_orders_count")]
    pub recent_orders_count: i64,
    /// Products shown in one top page section row.
    #[serde(default = "default_home_section_product_count")]
    pub home_section_product_count: i64,
    /// Categories shown in the top page's category nav.
    #[serde(default = "default_home_category_nav_count")]
    pub home_category_nav_count: i64,
    /// Maximum addresses one member may save in the address
    /// book (mypage adds and checkout saves share the cap).
    #[serde(default = "default_max_addresses")]
    pub max_addresses: i64,
    /// Consumption-tax fraction handling: "floor" (切り捨て),
    /// "round" (四捨五入) or "ceil" (切り上げ), applied once per
    /// tax class to the class subtotal, never per line.
    #[serde(default = "default_tax_rounding")]
    pub tax_rounding: String,
    /// Stock level at or below which a product reads as low
    /// stock (dashboard warning and the 残りわずか label) when
    /// the product carries no threshold of its own.
    #[serde(default = "default_low_stock_default")]
    pub low_stock_default: i64,
    /// Base URL of the OpenAI-compatible chat API the shopping
    /// agent's translator talks to.
    /// The default is the local Ollama server; a cloud provider
    /// is a config change, not a code change. The API key, when
    /// the provider needs one, is NOT here: it comes from the
    /// `LLM_API_KEY` environment variable only.
    #[serde(default = "default_llm_base_url")]
    pub llm_base_url: String,
    /// Model name the translator requests at `llm_base_url`.
    #[serde(default = "default_llm_model")]
    pub llm_model: String,
    /// Seconds one translator call may take before the chat
    /// answers with its fallback message instead.
    #[serde(default = "default_llm_timeout_secs")]
    pub llm_timeout_secs: u64,
    /// Most product lines the translator's grounding digest
    /// may carry for one category (cost trim).
    /// The digest rides along on every translator call, so
    /// this caps the worst case when one category holds a
    /// large share of the catalogue.
    #[serde(default = "default_llm_digest_max_products")]
    pub llm_digest_max_products: usize,
    /// Whether `/agent` serves the conversational chat
    /// interface. Off, the page is the structured brief form —
    /// the fallback face, and the one the form suite exercises.
    /// Minutes an agent-checkout approval stays usable
    /// the window between the owner approving
    /// the cart and the placement re-check. Approving binds
    /// the cart's fingerprint, never its stock, so the TTL
    /// only bounds how long the binding may wait.
    #[serde(default = "default_agent_approval_ttl_minutes")]
    pub agent_approval_ttl_minutes: i64,
    /// Approval creations one cart owner may make per hour
    ///, counted in the run limiter's pattern.
    #[serde(default = "default_agent_checkout_max_per_hour")]
    pub agent_checkout_max_per_hour: i64,
    /// How long a member session may sit idle before it
    /// expires, as a duration with a unit suffix (`s`, `m`,
    /// `h` or `d`, for example "14d"), parsed by
    /// [`parse_ttl`]. The expiry is sliding (OnInactivity):
    /// every request inside the window extends it by the
    /// full span, so this is a silence limit, not a session
    /// age limit.
    #[serde(default = "default_member_session_ttl")]
    pub member_session_ttl: String,
    /// How long an admin session may sit idle before it
    /// expires, in the same duration form (for example
    /// "8h"; sliding, as the member session). Far shorter
    /// than the member span: the back office is the
    /// privileged surface.
    #[serde(default = "default_admin_session_ttl")]
    pub admin_session_ttl: String,
    /// How often the expired-session cleanup deletes
    /// session rows past their expiry, in the same
    /// duration form (for example "20m"). The lifetimes
    /// above decide when a session dies; this only
    /// decides how soon its row leaves the table.
    #[serde(default = "default_session_cleanup_interval")]
    pub session_cleanup_interval: String,
    /// Source IPs allowed to reach the `/admin` surface, as
    /// exact addresses or CIDR ranges (for example
    /// `"203.0.113.7"` or `"203.0.113.0/24"`), enforced by the
    /// admin IP guard before any session work. The default is
    /// loopback only: the back office stays unreachable from
    /// the network until the operator lists an address.
    #[serde(default = "default_admin_allowed_ips")]
    pub admin_allowed_ips: Vec<String>,
}

/// Returns the default admin allowlist: this machine only.
/// The operator's own addresses are listed explicitly in
/// `config.toml`; until then the back office is local-only.
fn default_admin_allowed_ips() -> Vec<String> {
    vec!["127.0.0.1".to_string(), "::1".to_string()]
}

/// Returns the default consumption-tax rounding mode: floor
/// (切り捨て), the Japanese invoice convention.
fn default_tax_rounding() -> String {
    "floor".to_string()
}

/// Returns the default low-stock threshold: three units, the
/// value the dashboard and labels hard-coded before the setting
/// existed.
fn default_low_stock_default() -> i64 {
    3
}

/// Returns the default LLM endpoint: the local Ollama server's
/// OpenAI-compatible API. A cloud provider is selected by
/// pointing this at its own `/v1` base URL instead.
fn default_llm_base_url() -> String {
    "http://localhost:11434/v1".to_string()
}

/// Returns the default LLM model: the lightest local model
/// judged able to extract a brief from Japanese free text.
fn default_llm_model() -> String {
    "qwen2.5:1.5b-instruct".to_string()
}

/// Returns the default LLM call timeout in seconds: a local
/// model answers in a few seconds; sixty leaves room for a
/// cold model load without hanging the request thread forever.
fn default_llm_timeout_secs() -> u64 {
    60
}

/// Returns the default per-category digest cap: enough lines
/// for the translator to phrase needs in the store's own
/// words, small enough that one large category cannot
/// dominate every call's prompt.
fn default_llm_digest_max_products() -> usize {
    40
}

/// Returns the default approval TTL: twenty minutes, long
/// enough to finish a checkout, short enough that an
/// abandoned approval never lingers.
fn default_agent_approval_ttl_minutes() -> i64 {
    20
}

/// Returns the default approval cap: ten creations per owner
/// per hour, the same allowance the agent runs get.
fn default_agent_checkout_max_per_hour() -> i64 {
    10
}

/// Returns the default member session idle lifetime:
/// twenty minutes. This is only the fallback for a
/// config file that omits the key — the deployed
/// config.toml sets the storefront's real lifetime.
fn default_member_session_ttl() -> String {
    "20m".to_string()
}

/// Returns the default admin session idle lifetime:
/// twenty minutes, the same fallback as the member
/// lifetime; the deployed config.toml sets the back
/// office's real lifetime.
fn default_admin_session_ttl() -> String {
    "20m".to_string()
}

/// Returns the default expired-session cleanup interval:
/// twenty minutes, so a dead session's row never lingers
/// long in the store's largest table.
fn default_session_cleanup_interval() -> String {
    "20m".to_string()
}

/// Database connection settings from `config.toml`.
///
/// The password is absent on purpose: it comes from the
/// `DB_PASSWORD` environment variable, so no secret is committed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    /// Database host, for example localhost.
    pub host: String,
    /// Database port, 5432 for PostgreSQL.
    pub port: u16,
    /// Database name.
    pub name: String,
    /// Database user.
    pub user: String,
    /// Runtime database user: the least-privilege role the
    /// server connects as once migrations have run as `user`
    /// (the owner). Its password comes from the
    /// `APP_DB_PASSWORD` environment variable.
    #[serde(default = "default_db_app_user")]
    pub app_user: String,
}

impl Config {
    /// The member session idle lifetime as a duration,
    /// parsed from `member_session_ttl`.
    ///
    /// # Panics
    ///
    /// Panics on an unparsable value — unreachable in
    /// practice, because `validate` runs at load and
    /// rejects it there first.
    pub fn member_session_ttl_duration(&self) -> std::time::Duration {
        parse_ttl(&self.member_session_ttl).expect("member_session_ttl validated at load")
    }

    /// The admin session idle lifetime as a duration,
    /// parsed from `admin_session_ttl`. Same load-time
    /// validation guarantee as the member lifetime.
    pub fn admin_session_ttl_duration(&self) -> std::time::Duration {
        parse_ttl(&self.admin_session_ttl).expect("admin_session_ttl validated at load")
    }

    /// The expired-session cleanup interval as a
    /// duration, parsed from `session_cleanup_interval`.
    /// Same load-time validation guarantee as the
    /// session lifetimes.
    pub fn session_cleanup_interval_duration(&self) -> std::time::Duration {
        parse_ttl(&self.session_cleanup_interval)
            .expect("session_cleanup_interval validated at load")
    }

    /// Checks the loaded values whose meaning the code assumes.
    ///
    /// The stringly-typed settings are the dangerous ones: an
    /// unrecognised `db_ssl_mode` reads as "disable" and an
    /// unrecognised `tax_rounding` as "floor" at their use sites,
    /// so a typo would silently weaken the deployment. Page sizes
    /// are divisors as well as limits, so they must be positive.
    ///
    /// # Panics
    ///
    /// Panics naming the offending key: a misconfigured
    /// deployment must not start silently.
    pub fn validate(&self) {
        let mut problems: Vec<String> = Vec::new();
        if !matches!(self.db_ssl_mode.as_str(), "disable" | "require") {
            problems.push(format!(
                "db_ssl_mode must be \"disable\" or \"require\", got \"{}\"",
                self.db_ssl_mode
            ));
        }
        if !matches!(self.tax_rounding.as_str(), "floor" | "round" | "ceil") {
            problems.push(format!(
                "tax_rounding must be \"floor\", \"round\" or \"ceil\", got \"{}\"",
                self.tax_rounding
            ));
        }
        if !matches!(self.yen_symbol.as_str(), "￥" | "円") {
            problems.push(format!(
                "yen_symbol must be \"￥\" or \"円\", got \"{}\"",
                self.yen_symbol
            ));
        }
        for (key, value) in [
            ("admin_per_page", self.admin_per_page),
            ("search_per_page", self.search_per_page),
            ("api_per_page", self.api_per_page),
            ("csv_max_rows", self.csv_max_rows),
            ("max_addresses", self.max_addresses),
            ("cart_reminder_delay_hours", self.cart_reminder_delay_hours),
            ("csv_max_file_bytes", self.csv_max_file_bytes),
            ("reviews_per_page", self.reviews_per_page),
            ("history_per_page", self.history_per_page),
            ("recent_orders_count", self.recent_orders_count),
            (
                "home_section_product_count",
                self.home_section_product_count,
            ),
            ("home_category_nav_count", self.home_category_nav_count),
            (
                "agent_approval_ttl_minutes",
                self.agent_approval_ttl_minutes,
            ),
            (
                "agent_checkout_max_per_hour",
                self.agent_checkout_max_per_hour,
            ),
        ] {
            if value < 1 {
                problems.push(format!("{key} must be at least 1, got {value}"));
            }
        }
        for (key, value) in [
            ("member_session_ttl", &self.member_session_ttl),
            ("admin_session_ttl", &self.admin_session_ttl),
            ("session_cleanup_interval", &self.session_cleanup_interval),
        ] {
            if parse_ttl(value).is_none() {
                problems.push(format!(
                    "{key} must be a duration like \"30m\", \"8h\" or \"14d\", got \"{value}\""
                ));
            }
        }
        if self.low_stock_default < 0 {
            problems.push(format!(
                "low_stock_default must be 0 or more, got {}",
                self.low_stock_default
            ));
        }
        if self.llm_timeout_secs < 1 {
            problems.push(format!(
                "llm_timeout_secs must be at least 1, got {}",
                self.llm_timeout_secs
            ));
        }
        if !problems.is_empty() {
            for problem in &problems {
                eprintln!("Invalid configuration: {problem}");
            }
            panic!("Invalid configuration");
        }
    }

    /// The bootstrap admin's email from `ADMIN_EMAIL`, when set and
    /// non-empty.
    pub fn admin_email(&self) -> Option<String> {
        non_empty_env("ADMIN_EMAIL")
    }

    /// The bootstrap admin's password from `ADMIN_PASSWORD`, when
    /// set and non-empty.
    pub fn admin_password(&self) -> Option<String> {
        non_empty_env("ADMIN_PASSWORD")
    }

    /// The KOMOJU API secret key from `KOMOJU_SECRET_KEY`, when
    /// set and non-empty. Server-side only: never logged, never
    /// sent to a browser.
    pub fn komoju_secret_key(&self) -> Option<String> {
        non_empty_env("KOMOJU_SECRET_KEY")
    }

    /// The chat API key from `LLM_API_KEY`, when set and
    /// non-empty: sent as the Bearer credential to the
    /// configured `llm_base_url`. The local Ollama endpoint
    /// needs no key, so unset is the normal local state.
    /// Server-side only: never logged, never sent to a browser.
    pub fn llm_api_key(&self) -> Option<String> {
        non_empty_env("LLM_API_KEY")
    }

    /// The webhook secret token from `KOMOJU_WEBHOOK_SECRET`:
    /// the key the webhook HMAC is verified with.
    pub fn komoju_webhook_secret(&self) -> Option<String> {
        non_empty_env("KOMOJU_WEBHOOK_SECRET")
    }

    /// The Stripe API secret key from `STRIPE_SECRET_KEY`, when
    /// set and non-empty. Server-side only: never logged, never
    /// sent to a browser.
    pub fn stripe_secret_key(&self) -> Option<String> {
        non_empty_env("STRIPE_SECRET_KEY")
    }

    /// The webhook signing secret from `STRIPE_WEBHOOK_SECRET`:
    /// the key the Stripe-Signature HMAC is verified with.
    pub fn stripe_webhook_secret(&self) -> Option<String> {
        non_empty_env("STRIPE_WEBHOOK_SECRET")
    }

    /// The Stripe API base URL from `STRIPE_API_BASE_URL`,
    /// defaulting to the production API. The override exists so
    /// the test suites can point the client at a local stub.
    pub fn stripe_api_base_url(&self) -> String {
        non_empty_env("STRIPE_API_BASE_URL").unwrap_or_else(|| "https://api.stripe.com".to_string())
    }

    /// The store's public base URL from `PUBLIC_BASE_URL`, used
    /// to build the KOMOJU return URL. KOMOJU must be able to
    /// send the customer's browser back to it.
    pub fn public_base_url(&self) -> Option<String> {
        non_empty_env("PUBLIC_BASE_URL")
    }

    /// The Google Search Console verification token from
    /// `GOOGLE_SITE_VERIFICATION`, present when the operator has
    /// claimed the store in Search Console. The storefront layout
    /// renders it as the `google-site-verification` meta tag;
    /// unset renders nothing.
    pub fn google_site_verification(&self) -> Option<String> {
        non_empty_env("GOOGLE_SITE_VERIFICATION")
    }

    /// LINE Login channel id, from the environment only.
    #[must_use]
    pub fn line_channel_id(&self) -> Option<String> {
        non_empty_env("LINE_CHANNEL_ID")
    }

    /// LINE Login channel secret, from the environment only.
    #[must_use]
    pub fn line_channel_secret(&self) -> Option<String> {
        non_empty_env("LINE_CHANNEL_SECRET")
    }

    /// Google OAuth client id, from the environment only.
    #[must_use]
    pub fn google_client_id(&self) -> Option<String> {
        non_empty_env("GOOGLE_CLIENT_ID")
    }

    /// Google OAuth client secret, from the environment only.
    #[must_use]
    pub fn google_client_secret(&self) -> Option<String> {
        non_empty_env("GOOGLE_CLIENT_SECRET")
    }

    /// Yahoo! ID client id, from the environment only.
    #[must_use]
    pub fn yahoo_client_id(&self) -> Option<String> {
        non_empty_env("YAHOO_CLIENT_ID")
    }

    /// Yahoo! ID client secret, from the environment only.
    #[must_use]
    pub fn yahoo_client_secret(&self) -> Option<String> {
        non_empty_env("YAHOO_CLIENT_SECRET")
    }

    /// Base URL override for the LINE endpoints (test stubs);
    /// production endpoints apply when unset.
    #[must_use]
    pub fn sns_line_base_url(&self) -> Option<String> {
        non_empty_env("SNS_LINE_BASE_URL")
    }

    /// Base URL override for the Google endpoints (test stubs);
    /// production endpoints apply when unset.
    #[must_use]
    pub fn sns_google_base_url(&self) -> Option<String> {
        non_empty_env("SNS_GOOGLE_BASE_URL")
    }

    /// Base URL override for the Yahoo endpoints (test stubs);
    /// production endpoints apply when unset.
    #[must_use]
    pub fn sns_yahoo_base_url(&self) -> Option<String> {
        non_empty_env("SNS_YAHOO_BASE_URL")
    }

    /// The KOMOJU API base URL from `KOMOJU_API_BASE_URL`,
    /// defaulting to the production API. The override exists so
    /// the test suites can point the client at a local stub.
    pub fn komoju_api_base_url(&self) -> String {
        non_empty_env("KOMOJU_API_BASE_URL").unwrap_or_else(|| "https://komoju.com".to_string())
    }

    /// The bootstrap admin's display name from `ADMIN_DISPLAY_NAME`,
    /// defaulting to 管理者 when unset.
    ///
    /// The `admins` table requires a display name, which the spec's
    /// email/password pair does not provide.
    pub fn admin_display_name(&self) -> String {
        non_empty_env("ADMIN_DISPLAY_NAME").unwrap_or_else(|| "管理者".to_string())
    }

    /// Returns the PII field-encryption master key: 32 bytes,
    /// base64 in the `PII_ENCRYPTION_KEY` environment variable.
    ///
    /// # Panics
    ///
    /// Panics when the variable is missing or malformed: the
    /// store fails closed rather than start with personal data
    /// it can neither protect nor read back.
    pub fn pii_encryption_key(&self) -> [u8; 32] {
        let raw = non_empty_env("PII_ENCRYPTION_KEY")
            .expect("PII_ENCRYPTION_KEY must be set (base64 of 32 bytes)");
        let bytes = BASE64
            .decode(raw.trim())
            .expect("PII_ENCRYPTION_KEY must be valid base64");
        <[u8; 32]>::try_from(bytes.as_slice())
            .expect("PII_ENCRYPTION_KEY must decode to exactly 32 bytes")
    }
}

/// Reads an environment variable, treating an empty value as
/// unset, so a blank secret fails validation instead of becoming
/// an empty credential.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The single shared config instance, loaded on first access.
///
/// # Panics
///
/// Panics when `config.toml` is missing or invalid: a misconfigured
/// deployment must not start silently.
pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    // CONFIG_PATH wins when set: the deployed binary runs away from
    // the build workspace, where the compile-time path would not exist.
    let config_path = std::env::var_os("CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
            let workspace_root = manifest_dir.parent().unwrap_or(manifest_dir);
            workspace_root.join("config.toml")
        });

    eprintln!("Loading configuration file: {}", config_path.display());

    let text = fs::read_to_string(&config_path).unwrap_or_else(|e| {
        eprintln!(
            "Failed to read configuration file {}: {}",
            config_path.display(),
            e
        );
        panic!("Unable to continue without configuration");
    });

    let config: Config = toml::from_str(&text).unwrap_or_else(|e| {
        eprintln!(
            "Failed to parse configuration file {}: {}",
            config_path.display(),
            e
        );
        panic!("Invalid configuration");
    });

    config.validate();
    eprintln!("Loaded configuration from {}", config_path.display());
    config
});

/// Parses a TTL setting: a positive integer with one unit
/// suffix — `s` seconds, `m` minutes, `h` hours, `d` days
/// ("30m", "8h", "14d"). `None` for anything else, zero
/// included: a zero lifetime would expire every session on
/// sight, which reads as a typo, never an intent.
pub fn parse_ttl(value: &str) -> Option<std::time::Duration> {
    let (digits, unit_secs) = match value.as_bytes().last()? {
        b's' => (&value[..value.len() - 1], 1),
        b'm' => (&value[..value.len() - 1], 60),
        b'h' => (&value[..value.len() - 1], 3_600),
        b'd' => (&value[..value.len() - 1], 86_400),
        _ => return None,
    };
    let amount: u64 = digits.parse().ok()?;
    if amount == 0 {
        return None;
    }
    Some(std::time::Duration::from_secs(
        amount.checked_mul(unit_secs)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_ttl;
    use std::time::Duration;

    #[test]
    fn parses_each_unit() {
        assert_eq!(parse_ttl("45s"), Some(Duration::from_secs(45)));
        assert_eq!(parse_ttl("30m"), Some(Duration::from_secs(1_800)));
        assert_eq!(parse_ttl("8h"), Some(Duration::from_secs(28_800)));
        assert_eq!(parse_ttl("14d"), Some(Duration::from_secs(1_209_600)));
    }

    #[test]
    fn rejects_anything_else() {
        assert_eq!(parse_ttl(""), None);
        assert_eq!(parse_ttl("14"), None);
        assert_eq!(parse_ttl("14x"), None);
        assert_eq!(parse_ttl("0m"), None);
        assert_eq!(parse_ttl("-5m"), None);
        assert_eq!(parse_ttl("1.5h"), None);
    }
}
