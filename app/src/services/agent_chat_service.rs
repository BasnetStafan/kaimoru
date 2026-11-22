//! Shopping-agent chat brain.
//!
//! One visitor message gets one call to an OpenAI-compatible
//! chat API (the local Ollama server by default), which
//! answers with a structured decision: ask a question, run a
//! search, or just reply — plus the brief updates the message
//! carried. The model never sees or states product facts: the
//! system prompt is the versioned instruction file plus the
//! store's real vocabulary (categories, tags, a product
//! digest) as grounding, and every product claim on the page
//! comes from the deterministic engine's output, not from the
//! model. The conversation itself — thread entries and the
//! accumulated brief — lives in the visitor's session.

use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::config::CONFIG;
use crate::models::agent::{ChatBrief, Preset};
use crate::repositories::tag_repository;
use crate::services::agent_run_service::{AgentPick, AgentProposal, OverBudgetPick};
use crate::services::{agent_chat_rules, api_catalog_service, product_service};

/// The standing instruction set for the translator: a
/// versioned file in the repo, never editable at runtime —
/// an editable prompt would be an injection surface.
const PROMPT: &str = include_str!("agent_chat_prompt.md");

/// The agent's greeting on an empty thread: a constant, not
/// a model call, so the first page load is instant and free.
pub const GREETING: &str = "いらっしゃいませ。お買い物のお手伝いをいたします。お探しのものや、ご予算・こだわりがあれば教えてください。";

/// The agent's thread message when the translator cannot be
/// reached or answers unusable output: the conversation
/// carries on, the brief is untouched.
pub const FALLBACK_MESSAGE: &str =
    "申し訳ございません、少し調子が悪いようです。お手数ですが、もう一度お試しください。";

/// The agent's closing line when a conversation ends on the
/// visitor's thanks but the model's decision was not a plain
/// reply (its message would have promised a search or asked
/// another question): a fixed polite sign-off, so the words
/// are the shop's, not an improvisation.
pub const CLOSING_MESSAGE: &str =
    "ありがとうございました。またお探しのものがあれば、お気軽にお声がけください。";

/// Clarifying questions the agent may ask in a row before
/// it must search with whatever the brief already holds
/// (enforced in [`enforce_rules`]). Asking is free of the
/// run cap, but it is not free of the visitor's patience.
const MAX_QUESTIONS_SINCE_SEARCH: u8 = 2;

/// Example prompts offered as chips on an empty thread: one
/// fully-specified brief, one vague wish, one priority-led
/// ask — the three shapes a first message usually takes.
pub const EXAMPLE_PROMPTS: [&str; 3] = [
    "冬のキャンプで使える、軽くて暖かいものが欲しいです。予算は1万円まで",
    "仕事で使える丈夫なグッズを探しています",
    "安くて評価の高いものを教えてください",
];

/// Thread entries kept in the session: enough for a long
/// consultation, bounded so the session record cannot grow
/// without limit. Entries are display state only — dropping
/// the oldest loses nothing but scrollback.
const MAX_THREAD_ENTRIES: usize = 40;

/// What the translator decided to do with one message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatAction {
    /// Ask one clarifying question; no search runs.
    Ask,
    /// Run the engine with the accumulated brief.
    Search,
    /// Just talk; no search runs.
    Reply,
}

/// A budget update: the two ways a message can move the
/// accumulated budget. Absence of an update — the field the
/// message never mentioned — is `None` on [`BriefUpdates`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetUpdate {
    /// A stated cap, in yen (always positive).
    Set(i64),
    /// The visitor waived the budget （予算は気にしない）:
    /// the model signals this with `budget_yen: 0`.
    Clear,
}

/// A category update, with the same keep/set/clear split as
/// the budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CategoryUpdate {
    /// Restrict the search to this category (validated
    /// against the store's real category names).
    Set(String),
    /// Drop the restriction.
    Clear,
}

/// The brief changes one message carried, already validated.
/// A `None` field means "not mentioned — keep the accumulated
/// value"; the merge in [`merge_brief`] is the only place
/// updates touch the accumulated brief.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BriefUpdates {
    /// Budget change, when the message mentioned money.
    pub budget_yen: Option<BudgetUpdate>,
    /// Category change, when the message named one.
    pub category: Option<CategoryUpdate>,
    /// Replacement must-have needs, when the message stated
    /// any (an empty list clears the accumulated ones).
    pub must_have: Option<Vec<String>>,
    /// Replacement nice-to-have needs, same rule.
    pub nice_to_have: Option<Vec<String>>,
    /// Preset change, when the message expressed a priority.
    pub preset: Option<Preset>,
}

/// One validated translator decision.
#[derive(Clone, Debug)]
pub struct ChatDecision {
    /// What to do with this message.
    pub action: ChatAction,
    /// The agent's chat text for this turn, in Japanese.
    pub message: String,
    /// The brief updates the message carried.
    pub updates: BriefUpdates,
}

/// Applies validated updates to the accumulated brief: set
/// replaces, clear empties, absent keeps. Pure, so the merge
/// rules are unit-testable without a model.
pub fn merge_brief(brief: &mut ChatBrief, updates: &BriefUpdates) {
    match updates.budget_yen {
        Some(BudgetUpdate::Set(yen)) => brief.budget_yen = Some(yen),
        Some(BudgetUpdate::Clear) => brief.budget_yen = None,
        None => {}
    }
    match &updates.category {
        Some(CategoryUpdate::Set(name)) => brief.category = Some(name.clone()),
        Some(CategoryUpdate::Clear) => brief.category = None,
        None => {}
    }
    if let Some(needs) = &updates.must_have {
        brief.must_have = needs.clone();
    }
    if let Some(needs) = &updates.nice_to_have {
        brief.nice_to_have = needs.clone();
    }
    if let Some(preset) = updates.preset {
        brief.preset = preset;
        brief.preset_explicit = true;
    }
}

/// Removes one accumulated condition at the visitor's direct
/// instruction (a brief chip's clear button — no translator
/// involved): `field` names the brief slot ("budget",
/// "category", "must_have", "nice_to_have") and `value` the
/// need text for the list slots. Returns a short phrase
/// naming what was removed, for the thread's note, or `None`
/// when nothing matched (a stale chip, a forged post) and
/// the brief is untouched.
pub fn clear_brief_item(brief: &mut ChatBrief, field: &str, value: &str) -> Option<String> {
    match field {
        "budget" => brief.budget_yen.take().map(|_| "予算の条件".to_string()),
        "category" => brief
            .category
            .take()
            .map(|name| format!("カテゴリー「{name}」")),
        "must_have" | "nice_to_have" => {
            let needs = if field == "must_have" {
                &mut brief.must_have
            } else {
                &mut brief.nice_to_have
            };
            let before = needs.len();
            needs.retain(|need| need != value);
            (needs.len() != before).then(|| format!("条件「{value}」"))
        }
        _ => None,
    }
}

/// One entry of the conversation thread, as stored in the
/// session and rendered by the chat page.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatEntry {
    /// A message the visitor sent.
    User {
        /// The message text, as typed.
        text: String,
    },
    /// A chat message from the agent (a question, a reply, or
    /// the fallback notice after a failed translator call).
    Agent {
        /// The message text.
        text: String,
    },
    /// A search the agent ran: its lead-in text plus the
    /// snapshot the product cards render from.
    Result {
        /// The agent's lead-in text for this search.
        message: String,
        /// The run's outcome, snapshotted at run time.
        snapshot: ChatResultSnapshot,
    },
}

/// The part of one engine proposal the thread keeps: the
/// picks and refusals with their explanations, and the
/// projected totals if the shopper applies the picks. Card
/// data (images, ratings) is re-read at render time; this
/// record is the proposal's own facts. A snapshot is a
/// proposal, never a cart state: nothing in it was written
/// to the cart by the search that produced it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatResultSnapshot {
    /// The budget the proposal enforced (`None`: no cap).
    pub budget_yen: Option<i64>,
    /// Picks the proposal offers for the shopper to apply.
    pub picks: Vec<AgentPick>,
    /// Picks the budget refused.
    pub over_budget: Vec<OverBudgetPick>,
    /// Picks skipped as already in the cart by hand.
    pub already_in_cart: Vec<AgentPick>,
    /// Must-haves no pick satisfied by tag.
    pub unmet_needs: Vec<String>,
    /// Projected cart subtotal with the picks applied, in yen.
    pub subtotal_yen: i64,
    /// Projected shipping with the picks applied, in yen.
    pub shipping_yen: i64,
    /// Projected cart total with the picks applied, in yen.
    pub total_yen: i64,
    /// Retired with the proposal/apply split: runs once
    /// reported leaving the previous proposal's lines in
    /// place. Proposals now never touch the cart at all, so
    /// this is always false; the field stays so threads
    /// stored before the split still load.
    pub kept_previous: bool,
}

impl ChatResultSnapshot {
    /// Snapshots a finished proposal for the thread.
    pub fn from_proposal(proposal: &AgentProposal, budget_yen: Option<i64>) -> Self {
        Self {
            budget_yen,
            picks: proposal.picks.clone(),
            over_budget: proposal.over_budget.clone(),
            already_in_cart: proposal.already_in_cart.clone(),
            unmet_needs: proposal.unmet_needs.clone(),
            subtotal_yen: proposal.projected_totals.subtotal_yen,
            shipping_yen: proposal.projected_totals.shipping_yen,
            total_yen: proposal.projected_totals.total_yen,
            kept_previous: false,
        }
    }
}

/// The whole conversation state kept in the session: the
/// thread plus the brief it has accumulated.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChatState {
    /// Thread entries, oldest first.
    pub entries: Vec<ChatEntry>,
    /// The accumulated brief.
    pub brief: ChatBrief,
    /// Clarifying questions asked since the last search ran:
    /// the question budget's counter ([`enforce_rules`]).
    /// Sessions stored before the counter existed read as 0.
    #[serde(default)]
    pub asks_since_search: u8,
    /// Whether the previous turn ended by asking a question:
    /// the confirmation rule's trigger. Sessions stored
    /// before the flag existed read as false.
    #[serde(default)]
    pub last_turn_was_question: bool,
}

impl ChatState {
    /// Appends an entry, dropping the oldest entries past
    /// the thread cap.
    pub fn push(&mut self, entry: ChatEntry) {
        self.entries.push(entry);
        if self.entries.len() > MAX_THREAD_ENTRIES {
            let excess = self.entries.len() - MAX_THREAD_ENTRIES;
            self.entries.drain(..excess);
        }
    }
}

/// Why a translator call produced no decision. The variants
/// carry diagnostics for the log only — never a response
/// body, a prompt, or anything the visitor typed.
#[derive(Debug)]
pub enum ChatError {
    /// The endpoint could not be reached or answered an
    /// error status.
    Unreachable(String),
    /// The endpoint answered, but not with a usable decision.
    BadOutput(String),
    /// The app's own grounding reads failed.
    Internal(String),
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(detail) => write!(f, "llm endpoint unreachable: {detail}"),
            Self::BadOutput(detail) => write!(f, "llm output unusable: {detail}"),
            Self::Internal(detail) => write!(f, "agent chat internal failure: {detail}"),
        }
    }
}

impl std::error::Error for ChatError {}

/// The store vocabulary one call is grounded in.
struct Grounding {
    /// Real category names, in display order.
    categories: Vec<String>,
    /// Real tag labels.
    tag_labels: Vec<String>,
    /// One digest line per public product (name, price,
    /// tags), paired with the product's category name so the
    /// prompt builder can scope what a given call carries.
    digest: Vec<(String, String)>,
    /// One summary line per category that holds public
    /// products (count and price range): the grounding for a
    /// conversation that has not narrowed to a category yet.
    catalogue_summary: Vec<String>,
}

/// Reads the grounding from the catalog: categories and tags
/// by their own services, the product digest from the Session
/// 4 catalog service (all pages), so the model phrases needs
/// in words the matcher can actually find.
async fn grounding(pool: &PgPool) -> Result<Grounding, ChatError> {
    let categories = product_service::categories(pool)
        .await
        .map_err(|e| ChatError::Internal(e.to_string()))?;
    let tag_labels = tag_repository::list_tags(pool)
        .await
        .map_err(|e| ChatError::Internal(e.to_string()))?
        .into_iter()
        .map(|tag| tag.label)
        .collect();
    let first = api_catalog_service::list_products(pool, 1)
        .await
        .map_err(|e| ChatError::Internal(e.to_string()))?;
    let mut products = first.products;
    for page in 2..=first.total_pages {
        products.extend(
            api_catalog_service::list_products(pool, page)
                .await
                .map_err(|e| ChatError::Internal(e.to_string()))?
                .products,
        );
    }
    let digest = products
        .iter()
        .map(|product| {
            let tags = if product.tags.is_empty() {
                "なし".to_string()
            } else {
                product
                    .tags
                    .iter()
                    .map(|tag| tag.label.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            };
            (
                product.category.name.clone(),
                format!(
                    "・{}（{}円）タグ：{}",
                    product.name, product.price_yen, tags
                ),
            )
        })
        .collect();
    let catalogue_summary = categories
        .iter()
        .filter_map(|category| {
            let in_category: Vec<_> = products
                .iter()
                .filter(|product| &product.category.name == category)
                .collect();
            if in_category.is_empty() {
                return None;
            }
            let min = in_category.iter().map(|p| p.price_yen).min()?;
            let max = in_category.iter().map(|p| p.price_yen).max()?;
            let range = if min == max {
                format!("{min}円")
            } else {
                format!("{min}円〜{max}円")
            };
            Some(format!("・{category}：{}点（{range}）", in_category.len()))
        })
        .collect();
    Ok(Grounding {
        categories,
        tag_labels,
        digest,
        catalogue_summary,
    })
}

/// The preset's Japanese label, for the prompt's state block.
fn preset_label(preset: Preset) -> &'static str {
    match preset {
        Preset::Balanced => "バランス",
        Preset::Cheapest => "価格重視",
        Preset::BestRated => "評価重視",
    }
}

/// Assembles the system prompt: the versioned instruction
/// file, then the grounding (the store's real vocabulary and
/// the conversation-scoped product digest), then the
/// conversation's current brief state. Pure, so the assembly
/// is unit-testable.
pub fn build_system_prompt(
    categories: &[String],
    tag_labels: &[String],
    digest: &[(String, String)],
    catalogue_summary: &[String],
    brief: &ChatBrief,
    digest_max_products: usize,
) -> String {
    let budget = match brief.budget_yen {
        Some(yen) => format!("{yen}円まで"),
        None => "未定（上限なし）".to_string(),
    };
    let category = brief.category.as_deref().unwrap_or("指定なし");
    let must_have = if brief.must_have.is_empty() {
        "なし".to_string()
    } else {
        brief.must_have.join("、")
    };
    let nice_to_have = if brief.nice_to_have.is_empty() {
        "なし".to_string()
    } else {
        brief.nice_to_have.join("、")
    };
    // The product grounding is scoped to the conversation
    // (cost trim, 2026-11-10): the whole catalogue used to
    // ride along on every call. A conversation that has not
    // narrowed to a category gets one summary line per
    // category; a narrowed one gets that category's own
    // lines, capped so a large category cannot dominate the
    // prompt. A category whose lines are missing (emptied
    // since the brief was set) falls back to the summary —
    // the section is never sent empty.
    let cap = digest_max_products.max(1);
    let digest_block = match &brief.category {
        Some(category) => {
            let lines: Vec<&str> = digest
                .iter()
                .filter(|(owner, _)| owner == category)
                .map(|(_, line)| line.as_str())
                .collect();
            if lines.is_empty() {
                catalogue_summary.join("\n")
            } else if lines.len() > cap {
                let mut text = lines[..cap].join("\n");
                text.push_str(&format!("\n・（ほか{}点）", lines.len() - cap));
                text
            } else {
                lines.join("\n")
            }
        }
        None => catalogue_summary.join("\n"),
    };
    format!(
        "{PROMPT}\n\n【現在の条件】\n予算：{budget}\nカテゴリー：{category}\n必ず欲しい条件：{must_have}\nあれば嬉しい条件：{nice_to_have}\n重視するポイント：{}\n\n【カテゴリー一覧】\n{}\n\n【タグ一覧】\n{}\n\n【商品一覧（参考）】\n{}",
        preset_label(brief.preset),
        categories.join("、"),
        tag_labels.join("、"),
        digest_block,
    )
}

/// How one thread entry is replayed to the model: chat text
/// as-is; a search result as its lead-in plus a one-line
/// factual summary, so follow-ups ("cheaper?") have context
/// without the model ever inventing the facts itself.
fn replay_message(entry: &ChatEntry) -> Value {
    match entry {
        ChatEntry::User { text } => json!({"role": "user", "content": text}),
        ChatEntry::Agent { text } => json!({"role": "assistant", "content": text}),
        ChatEntry::Result { message, snapshot } => {
            let summary = if snapshot.picks.is_empty() {
                "（検索結果：条件に合う商品はカートに追加できませんでした）".to_string()
            } else {
                let names = snapshot
                    .picks
                    .iter()
                    .map(|pick| pick.name.as_str())
                    .collect::<Vec<_>>()
                    .join("、");
                format!(
                    "（検索結果：{}をカートに入れました。合計{}円）",
                    names, snapshot.total_yen
                )
            };
            json!({"role": "assistant", "content": format!("{message}\n{summary}")})
        }
    }
}

/// The shared HTTP client, built once with the configured
/// timeout. Resolution failures are the caller's to report.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(CONFIG.llm_timeout_secs.max(1)))
            .build()
            .unwrap_or_else(|e| {
                tracing::error!("llm client could not be built: {e}; using the default client");
                reqwest::Client::new()
            })
    })
}

/// Asks the translator for one decision: grounds the call in
/// the store's vocabulary, replays the thread, and validates
/// the answer before anything acts on it. Unusable output is
/// retried once with a correction hint before giving up.
///
/// # Errors
///
/// Returns [`ChatError`] when the endpoint is unreachable,
/// answers an error status, or answers unusable output; the
/// caller renders the fallback message and changes no state
/// beyond the thread itself.
pub async fn decide(
    pool: &PgPool,
    state: &ChatState,
    user_text: &str,
) -> Result<ChatDecision, ChatError> {
    let grounding = grounding(pool).await?;
    let system = build_system_prompt(
        &grounding.categories,
        &grounding.tag_labels,
        &grounding.digest,
        &grounding.catalogue_summary,
        &state.brief,
        CONFIG.llm_digest_max_products,
    );
    let mut messages = vec![json!({"role": "system", "content": system})];
    for entry in &state.entries {
        messages.push(replay_message(entry));
    }
    messages.push(json!({"role": "user", "content": user_text}));
    let content = call_model(&messages).await?;
    let mut decision = match agent_chat_rules::parse_decision(&content) {
        Ok(decision) => decision,
        Err(first_error) => {
            // One retry with a correction hint: the lightest
            // local models occasionally answer a vague message
            // with an empty object, and a bare retry wastes
            // the turn less often than the hinted one does.
            let mut retry_messages = messages.clone();
            retry_messages.push(json!({
                "role": "system",
                "content": "直前の出力は形式通りではありませんでした。必ず action・message・brief を含むJSONオブジェクト1つだけで答えてください。",
            }));
            let retried = call_model(&retry_messages).await?;
            agent_chat_rules::parse_decision(&retried).map_err(|_| first_error)?
        }
    };
    agent_chat_rules::sanitise_updates(&mut decision.updates, &grounding.categories);
    enforce_rules(&mut decision, state, user_text);
    Ok(decision)
}

/// The brief a decision would search with: the accumulated
/// brief with the decision's updates merged in. Pure, so the
/// rule pipeline below is unit-testable.
fn merged_brief(state: &ChatState, decision: &ChatDecision) -> ChatBrief {
    let mut merged = state.brief.clone();
    merge_brief(&mut merged, &decision.updates);
    merged
}

/// Returns whether a brief carries anything to search FOR:
/// a category, a need, or a preset chosen deliberately (a
/// chip tap, a model update, or the rules reading priority
/// words). A budget alone is a constraint, not a target, and
/// the untouched default preset is just the starting value —
/// neither one is a signal.
pub fn has_signal(brief: &ChatBrief) -> bool {
    brief.category.is_some()
        || !brief.must_have.is_empty()
        || !brief.nice_to_have.is_empty()
        || brief.preset_explicit
}

/// Returns whether this conversation has already run a
/// search: its thread carries a result entry.
fn has_searched(state: &ChatState) -> bool {
    state
        .entries
        .iter()
        .any(|entry| matches!(entry, ChatEntry::Result { .. }))
}

/// Applies the deterministic conversation rules to one
/// translator decision, in enforcement order. The model only
/// extracts language; wherever the situation is recognisable
/// the rules — whose pattern lists live in
/// [`agent_chat_rules`] — have the last word:
///
/// 1. Closing: thanks or a sign-off never searches and never
///    touches the brief; the model's text is kept only when
///    it was already a plain reply.
/// 2. Confirmation: the previous turn asked a question and
///    the visitor affirmed — search with the accumulated
///    brief, once it has signal.
/// 3. Follow-up transforms: adjustments stated in words
///    (cheaper / better rated / a yen amount) merge into the
///    brief, and after an earlier search they re-run it.
/// 4. Question budget: after `MAX_QUESTIONS_SINCE_SEARCH`
///    clarifying questions with no search between them, a
///    brief with any signal searches instead of asking
///    again; with no signal at all, asking is the only move
///    there is, so it continues.
/// 5. Promise rescue: a reply that promises a search becomes
///    the search it promised, once the brief has signal.
/// 6. The model's own action — with the signalless guard
///    last: a search with nothing to search for degrades to
///    a reply.
pub fn enforce_rules(decision: &mut ChatDecision, state: &ChatState, user_text: &str) {
    if agent_chat_rules::is_closing(user_text) {
        if decision.action != ChatAction::Reply {
            decision.message = CLOSING_MESSAGE.to_string();
        }
        decision.action = ChatAction::Reply;
        decision.updates = BriefUpdates::default();
        return;
    }
    if state.last_turn_was_question
        && agent_chat_rules::is_affirmation(user_text)
        && has_signal(&merged_brief(state, decision))
    {
        decision.action = ChatAction::Search;
    }
    if let Some(transform) = agent_chat_rules::follow_up_transform(user_text) {
        if let Some(preset) = transform.preset {
            decision.updates.preset = Some(preset);
        }
        if let Some(yen) = transform.budget_yen {
            decision.updates.budget_yen = Some(BudgetUpdate::Set(yen));
        }
        if decision.action != ChatAction::Search
            && has_searched(state)
            && has_signal(&merged_brief(state, decision))
        {
            decision.action = ChatAction::Search;
        }
    }
    if decision.action == ChatAction::Ask
        && state.asks_since_search >= MAX_QUESTIONS_SINCE_SEARCH
        && has_signal(&merged_brief(state, decision))
    {
        decision.action = ChatAction::Search;
    }
    if matches!(decision.action, ChatAction::Ask | ChatAction::Reply)
        && agent_chat_rules::promises_search(&decision.message)
        && has_signal(&merged_brief(state, decision))
    {
        decision.action = ChatAction::Search;
    }
    downgrade_signalless_search(decision, state);
}

/// Turns a "search" that carries nothing to search FOR into
/// a plain reply: with no category, no needs and no
/// deliberate preset, the engine would rank the whole
/// catalogue and add its top scorers to the cart unprompted
/// — the worst answer to a greeting. The model's message
/// still shows; the conversation continues and a later,
/// specified message can search for real.
fn downgrade_signalless_search(decision: &mut ChatDecision, state: &ChatState) {
    if decision.action != ChatAction::Search {
        return;
    }
    if !has_signal(&merged_brief(state, decision)) {
        decision.action = ChatAction::Reply;
    }
}

/// The decision contract as a JSON schema for endpoints
/// that take one. Every brief field is required-but-nullable:
/// null is the accumulated-brief "keep" signal, so a strict
/// schema never forces the model to invent or erase a value.
fn decision_format() -> Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "chat_decision",
            "strict": true,
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["action", "message", "brief"],
                "properties": {
                    "action": {"type": "string", "enum": ["ask", "search", "reply"]},
                    "message": {"type": "string"},
                    "brief": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["budget_yen", "category", "must_have", "nice_to_have", "preset"],
                        "properties": {
                            "budget_yen": {"type": ["integer", "null"]},
                            "category": {"type": ["string", "null"]},
                            "must_have": {"type": ["array", "null"], "items": {"type": "string"}},
                            "nice_to_have": {"type": ["array", "null"], "items": {"type": "string"}},
                            "preset": {"type": ["string", "null"], "enum": ["balanced", "cheapest", "best_rated", null]}
                        }
                    }
                }
            }
        }
    })
}

/// Why one chat-endpoint call failed: a 400 (the endpoint
/// rejected the request shape, e.g. an unknown response
/// format) is worth one retry in a plainer shape; a transient
/// failure (a transport error, or a 429/5xx status — the
/// shapes a load-shedding endpoint produces) is worth one
/// retry after a short backoff; anything else is final for
/// this call.
enum PostError {
    /// The endpoint answered 400 Bad Request.
    Rejected,
    /// A transient failure: worth exactly one backoff retry.
    /// `post_chat` converts a second transient failure into
    /// [`PostError::Failed`], so this never reaches callers.
    Transient(String),
    /// Any other failure, ready to return to the caller.
    Failed(ChatError),
}

/// Calls the configured chat endpoint once and returns the
/// answer's message content. The decision is requested as a
/// strict JSON schema first — small local models need the
/// grammar constraint to hold the contract at all — and an
/// endpoint that rejects that format (400) is retried once
/// with the plainer JSON-object format.
///
/// # Errors
///
/// Returns [`ChatError::Unreachable`] when the endpoint
/// cannot be reached or answers an error status, and
/// [`ChatError::BadOutput`] when the response is not the
/// expected chat-completion shape.
async fn call_model(messages: &[Value]) -> Result<String, ChatError> {
    match post_chat(messages, &decision_format()).await {
        Ok(content) => Ok(content),
        Err(PostError::Rejected) => post_chat(messages, &json!({"type": "json_object"}))
            .await
            .map_err(|error| match error {
                PostError::Rejected => ChatError::Unreachable("status 400 Bad Request".to_string()),
                PostError::Failed(error) => error,
                // Defensive: post_chat never returns Transient.
                PostError::Transient(detail) => ChatError::Unreachable(detail),
            }),
        Err(PostError::Failed(error)) => Err(error),
        // Defensive: post_chat never returns Transient.
        Err(PostError::Transient(detail)) => Err(ChatError::Unreachable(detail)),
    }
}

/// One chat-endpoint call with the given response format,
/// plus exactly one retry after a short backoff when the
/// first attempt fails transiently (transport error or a
/// 429/5xx status): free-tier endpoints shed load in brief
/// bursts — the live store saw Gemini 503s on 2026-11-10 —
/// and one retry turns most of them into a normal answer
/// instead of the chat's fallback message.
async fn post_chat(messages: &[Value], format: &Value) -> Result<String, PostError> {
    match post_chat_once(messages, format).await {
        Err(PostError::Transient(_)) => {
            tokio::time::sleep(Duration::from_millis(800)).await;
            post_chat_once(messages, format)
                .await
                .map_err(|error| match error {
                    PostError::Transient(detail) => {
                        PostError::Failed(ChatError::Unreachable(detail))
                    }
                    other => other,
                })
        }
        result => result,
    }
}

/// One raw chat-endpoint call with the given response format.
async fn post_chat_once(messages: &[Value], format: &Value) -> Result<String, PostError> {
    let body = json!({
        "model": CONFIG.llm_model,
        "messages": messages,
        "temperature": 0.2,
        "response_format": format,
        "stream": false,
    });
    let url = format!(
        "{}/chat/completions",
        CONFIG.llm_base_url.trim_end_matches('/')
    );
    let mut request = client().post(&url).json(&body);
    // The key exists only for providers that need one; the
    // local Ollama endpoint ignores the header entirely.
    if let Some(key) = CONFIG.llm_api_key() {
        request = request.bearer_auth(key);
    }
    let response = request
        .send()
        .await
        .map_err(|e| PostError::Transient(e.to_string()))?;
    let status = response.status();
    if status.as_u16() == 400 {
        return Err(PostError::Rejected);
    }
    if status.as_u16() == 429 || status.is_server_error() {
        return Err(PostError::Transient(format!("status {status}")));
    }
    if !status.is_success() {
        return Err(PostError::Failed(ChatError::Unreachable(format!(
            "status {status}"
        ))));
    }
    let payload: Value = response.json().await.map_err(|e| {
        PostError::Failed(ChatError::BadOutput(format!("response was not JSON: {e}")))
    })?;
    payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| {
            PostError::Failed(ChatError::BadOutput(
                "response carried no message content".to_string(),
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::{
        BriefUpdates, BudgetUpdate, CLOSING_MESSAGE, CategoryUpdate, ChatAction, ChatDecision,
        ChatEntry, ChatResultSnapshot, ChatState, build_system_prompt, clear_brief_item,
        downgrade_signalless_search, enforce_rules, has_signal, merge_brief,
    };
    use crate::models::agent::{ChatBrief, Preset};
    use crate::services::agent_chat_rules::{parse_decision, sanitise_updates};

    #[test]
    fn clearing_a_brief_item_removes_exactly_that_item() {
        let mut brief = ChatBrief {
            budget_yen: Some(5_000),
            category: Some("グッズ".to_string()),
            must_have: vec!["防水".to_string(), "軽い".to_string()],
            nice_to_have: vec!["青".to_string()],
            preset: Preset::Balanced,
            preset_explicit: false,
        };
        assert_eq!(
            clear_brief_item(&mut brief, "budget", ""),
            Some("予算の条件".to_string())
        );
        assert_eq!(brief.budget_yen, None);
        assert_eq!(
            clear_brief_item(&mut brief, "must_have", "防水"),
            Some("条件「防水」".to_string())
        );
        assert_eq!(brief.must_have, vec!["軽い".to_string()]);
        assert_eq!(
            clear_brief_item(&mut brief, "category", ""),
            Some("カテゴリー「グッズ」".to_string())
        );
        assert_eq!(brief.category, None);
        // Stale or forged posts change nothing.
        assert_eq!(clear_brief_item(&mut brief, "must_have", "防水"), None);
        assert_eq!(clear_brief_item(&mut brief, "unknown", "x"), None);
        assert_eq!(brief.nice_to_have, vec!["青".to_string()]);
    }

    #[test]
    fn parses_an_ask_decision_with_no_updates() {
        let decision =
            parse_decision(r#"{"action":"ask","message":"何をお探しですか？","brief":{}}"#)
                .expect("valid decision");
        assert_eq!(decision.action, ChatAction::Ask);
        assert_eq!(decision.message, "何をお探しですか？");
        assert_eq!(decision.updates, BriefUpdates::default());
    }

    #[test]
    fn parses_a_search_decision_with_brief_updates() {
        let decision = parse_decision(
r#"{"action":"search","message":"承知しました。お探しします。","brief":{"budget_yen":10000,"category":"グッズ","must_have":["絶対に軽い"],"nice_to_have":["防水"],"preset":"cheapest"}}"#,
)
.expect("valid decision");
        assert_eq!(decision.action, ChatAction::Search);
        assert_eq!(decision.updates.budget_yen, Some(BudgetUpdate::Set(10_000)));
        assert_eq!(
            decision.updates.category,
            Some(CategoryUpdate::Set("グッズ".to_string()))
        );
        assert_eq!(
            decision.updates.must_have,
            Some(vec!["絶対に軽い".to_string()])
        );
        assert_eq!(decision.updates.preset, Some(Preset::Cheapest));
    }

    #[test]
    fn fenced_json_still_parses() {
        let decision = parse_decision(
"```json\n{\"action\":\"reply\",\"message\":\"かしこまりました。\",\"brief\":null}\n```",
)
.expect("fenced decision");
        assert_eq!(decision.action, ChatAction::Reply);
    }

    #[test]
    fn unusable_output_is_refused() {
        assert!(parse_decision("すみません、よく分かりませんでした").is_err());
        assert!(parse_decision(r#"{"message":"action がありません"}"#).is_err());
        assert!(parse_decision(r#"{"action":"dance","message":"踊ります"}"#).is_err());
        assert!(parse_decision(r#"{"action":"ask","message":" "}"#).is_err());
    }

    #[test]
    fn a_search_without_a_message_gets_the_default_lead_in() {
        let decision =
            parse_decision(r#"{"action":"search","brief":{"budget_yen":5000}}"#).expect("search");
        assert_eq!(decision.message, "承知しました。お探しします。");
        assert_eq!(decision.updates.budget_yen, Some(BudgetUpdate::Set(5_000)));
    }

    #[test]
    fn budget_zero_clears_and_a_numeric_string_sets() {
        let cleared =
            parse_decision(r#"{"action":"search","message":"x","brief":{"budget_yen":0}}"#)
                .expect("clear");
        assert_eq!(cleared.updates.budget_yen, Some(BudgetUpdate::Clear));
        let set =
            parse_decision(r#"{"action":"search","message":"x","brief":{"budget_yen":"8000"}}"#)
                .expect("string budget");
        assert_eq!(set.updates.budget_yen, Some(BudgetUpdate::Set(8_000)));
        let negative =
            parse_decision(r#"{"action":"search","message":"x","brief":{"budget_yen":-100}}"#)
                .expect("negative ignored");
        assert_eq!(negative.updates.budget_yen, None);
    }

    #[test]
    fn merge_sets_clears_and_keeps() {
        let mut brief = ChatBrief {
            budget_yen: Some(5_000),
            category: Some("グッズ".to_string()),
            must_have: vec!["軽い".to_string()],
            nice_to_have: Vec::new(),
            preset: Preset::Balanced,
            ..ChatBrief::default()
        };
        // Absent fields keep.
        merge_brief(&mut brief, &BriefUpdates::default());
        assert_eq!(brief.budget_yen, Some(5_000));
        assert_eq!(brief.category.as_deref(), Some("グッズ"));
        // Zero budget clears; a new category replaces; a
        // present needs list replaces (even with another).
        merge_brief(
            &mut brief,
            &BriefUpdates {
                budget_yen: Some(BudgetUpdate::Clear),
                category: Some(CategoryUpdate::Set("アパレル".to_string())),
                must_have: Some(vec!["防水".to_string()]),
                nice_to_have: None,
                preset: Some(Preset::BestRated),
            },
        );
        assert_eq!(brief.budget_yen, None);
        assert_eq!(brief.category.as_deref(), Some("アパレル"));
        assert_eq!(brief.must_have, vec!["防水"]);
        assert_eq!(brief.preset, Preset::BestRated);
        // An empty category string clears the restriction.
        merge_brief(
            &mut brief,
            &BriefUpdates {
                category: Some(CategoryUpdate::Clear),
                ..BriefUpdates::default()
            },
        );
        assert_eq!(brief.category, None);
    }

    #[test]
    fn a_signalless_search_is_downgraded_to_a_reply() {
        // A budget alone is a constraint, not a target: the
        // greeting case must not run the engine.
        let state = ChatState::default();
        let mut decided = ChatDecision {
            action: ChatAction::Search,
            message: "テスト".to_string(),
            updates: BriefUpdates::default(),
        };
        decided.updates.budget_yen = Some(BudgetUpdate::Set(10_000));
        downgrade_signalless_search(&mut decided, &state);
        assert_eq!(decided.action, ChatAction::Reply);
    }

    #[test]
    fn a_search_with_a_need_survives_the_downgrade() {
        let state = ChatState::default();
        let mut decided = ChatDecision {
            action: ChatAction::Search,
            message: "テスト".to_string(),
            updates: BriefUpdates::default(),
        };
        decided.updates.nice_to_have = Some(vec!["軽い".to_string()]);
        downgrade_signalless_search(&mut decided, &state);
        assert_eq!(decided.action, ChatAction::Search);
    }

    #[test]
    fn accumulated_needs_keep_a_follow_up_search_alive() {
        // もっと安いのある？ carries no new need itself; the
        // accumulated brief is what it searches with.
        let mut state = ChatState::default();
        state.brief.must_have = vec!["防水".to_string()];
        let mut decided = ChatDecision {
            action: ChatAction::Search,
            message: "テスト".to_string(),
            updates: BriefUpdates::default(),
        };
        decided.updates.preset = Some(Preset::Cheapest);
        downgrade_signalless_search(&mut decided, &state);
        assert_eq!(decided.action, ChatAction::Search);
    }

    fn decision(action: ChatAction, message: &str) -> ChatDecision {
        ChatDecision {
            action,
            message: message.to_string(),
            updates: BriefUpdates::default(),
        }
    }

    fn result_entry() -> ChatEntry {
        ChatEntry::Result {
            message: "お探ししました。".to_string(),
            snapshot: ChatResultSnapshot {
                budget_yen: None,
                picks: Vec::new(),
                over_budget: Vec::new(),
                already_in_cart: Vec::new(),
                unmet_needs: Vec::new(),
                subtotal_yen: 0,
                shipping_yen: 0,
                total_yen: 0,
                kept_previous: false,
            },
        }
    }

    #[test]
    fn merge_marks_a_preset_update_explicit() {
        let mut brief = ChatBrief::default();
        assert!(!brief.preset_explicit);
        merge_brief(
            &mut brief,
            &BriefUpdates {
                preset: Some(Preset::Cheapest),
                ..BriefUpdates::default()
            },
        );
        assert!(brief.preset_explicit);
    }

    #[test]
    fn a_deliberate_preset_is_search_signal() {
        assert!(!has_signal(&ChatBrief::default()));
        let brief = ChatBrief {
            preset_explicit: true,
            ..ChatBrief::default()
        };
        assert!(has_signal(&brief));
    }

    #[test]
    fn an_affirmation_after_a_question_forces_the_search() {
        let state = ChatState {
            last_turn_was_question: true,
            brief: ChatBrief {
                preset: Preset::BestRated,
                preset_explicit: true,
                ..ChatBrief::default()
            },
            ..ChatState::default()
        };
        let mut decided = decision(ChatAction::Reply, "かしこまりました。");
        enforce_rules(&mut decided, &state, "はい、そういうものほしいです。");
        assert_eq!(decided.action, ChatAction::Search);
    }

    #[test]
    fn an_affirmation_without_signal_changes_nothing() {
        let state = ChatState {
            last_turn_was_question: true,
            ..ChatState::default()
        };
        let mut decided = decision(ChatAction::Reply, "かしこまりました。");
        enforce_rules(&mut decided, &state, "はい");
        assert_eq!(decided.action, ChatAction::Reply);
    }

    #[test]
    fn a_closing_turn_never_searches_and_keeps_the_brief() {
        let state = ChatState::default();
        let mut decided = decision(ChatAction::Search, "お探しします。");
        decided.updates.preset = Some(Preset::Cheapest);
        enforce_rules(&mut decided, &state, "ありがとうございました");
        assert_eq!(decided.action, ChatAction::Reply);
        assert_eq!(decided.message, CLOSING_MESSAGE);
        assert_eq!(decided.updates, BriefUpdates::default());
    }

    #[test]
    fn the_question_budget_forces_a_search_once_asking_is_spent() {
        let mut state = ChatState {
            asks_since_search: 2,
            ..ChatState::default()
        };
        state.brief.nice_to_have = vec!["軽い".to_string()];
        let mut decided = decision(ChatAction::Ask, "ご予算はどのくらいですか？");
        enforce_rules(&mut decided, &state, "友達へのものです");
        assert_eq!(decided.action, ChatAction::Search);
    }

    #[test]
    fn the_question_budget_keeps_asking_when_there_is_no_signal() {
        let state = ChatState {
            asks_since_search: 2,
            ..ChatState::default()
        };
        let mut decided = decision(ChatAction::Ask, "何をお探しですか？");
        enforce_rules(&mut decided, &state, "うーん");
        assert_eq!(decided.action, ChatAction::Ask);
    }

    #[test]
    fn a_promised_search_becomes_the_search() {
        let state = ChatState::default();
        let mut decided = decision(ChatAction::Reply, "軽いものを探しています。");
        decided.updates.preset = Some(Preset::Cheapest);
        enforce_rules(&mut decided, &state, "軽いものがほしいです");
        assert_eq!(decided.action, ChatAction::Search);
    }

    #[test]
    fn a_promise_without_signal_stays_a_reply() {
        let state = ChatState::default();
        let mut decided = decision(ChatAction::Reply, "お探ししています。");
        enforce_rules(&mut decided, &state, "こんにちは");
        assert_eq!(decided.action, ChatAction::Reply);
    }

    #[test]
    fn a_follow_up_adjustment_re_runs_the_previous_search() {
        let mut state = ChatState::default();
        state.entries.push(result_entry());
        state.brief.nice_to_have = vec!["軽い".to_string()];
        let mut decided = decision(ChatAction::Ask, "ほかに条件はありますか？");
        enforce_rules(&mut decided, &state, "もっと安いのありますか？");
        assert_eq!(decided.action, ChatAction::Search);
        assert_eq!(decided.updates.preset, Some(Preset::Cheapest));
    }

    #[test]
    fn a_signalless_search_is_still_downgraded_after_the_rules() {
        let state = ChatState::default();
        let mut decided = decision(ChatAction::Search, "お探しします。");
        enforce_rules(&mut decided, &state, "こんにちは");
        assert_eq!(decided.action, ChatAction::Reply);
    }

    #[test]
    fn an_unknown_category_update_is_dropped() {
        let mut updates = BriefUpdates {
            category: Some(CategoryUpdate::Set("存在しない".to_string())),
            ..BriefUpdates::default()
        };
        sanitise_updates(&mut updates, &["グッズ".to_string()]);
        assert_eq!(updates.category, None);
        let mut updates = BriefUpdates {
            category: Some(CategoryUpdate::Set("グッズ".to_string())),
            ..BriefUpdates::default()
        };
        sanitise_updates(&mut updates, &["グッズ".to_string()]);
        assert_eq!(
            updates.category,
            Some(CategoryUpdate::Set("グッズ".to_string()))
        );
    }

    #[test]
    fn the_system_prompt_carries_the_grounding_and_the_state() {
        let brief = ChatBrief {
            budget_yen: Some(10_000),
            must_have: vec!["軽い".to_string()],
            ..ChatBrief::default()
        };
        let prompt = build_system_prompt(
            &["アパレル".to_string(), "グッズ".to_string()],
            &["防水".to_string(), "軽量".to_string()],
            &[(
                "アパレル".to_string(),
                "・テスト商品（1000円）タグ：防水".to_string(),
            )],
            &["・アパレル：1点（1000円）".to_string()],
            &brief,
            40,
        );
        assert!(prompt.contains("アパレル"));
        assert!(prompt.contains("防水"));
        assert!(prompt.contains("10000円まで"));
        assert!(prompt.contains("JSON"));
        // No category yet: the summary rides along, the
        // per-product lines stay home (the cost trim).
        assert!(prompt.contains("・アパレル：1点（1000円）"));
        assert!(!prompt.contains("テスト商品"));
    }

    #[test]
    fn a_narrowed_brief_gets_only_its_category_lines() {
        let brief = ChatBrief {
            category: Some("アパレル".to_string()),
            ..ChatBrief::default()
        };
        let prompt = build_system_prompt(
            &["アパレル".to_string(), "グッズ".to_string()],
            &["防水".to_string()],
            &[
                (
                    "アパレル".to_string(),
                    "・テスト商品（1000円）タグ：防水".to_string(),
                ),
                (
                    "グッズ".to_string(),
                    "・別商品（2000円）タグ：なし".to_string(),
                ),
            ],
            &["・アパレル：1点（1000円）".to_string()],
            &brief,
            40,
        );
        assert!(prompt.contains("テスト商品"));
        assert!(!prompt.contains("別商品"));
    }

    #[test]
    fn the_digest_cap_truncates_with_a_remainder_line() {
        let brief = ChatBrief {
            category: Some("アパレル".to_string()),
            ..ChatBrief::default()
        };
        let digest: Vec<(String, String)> = (1..=3)
            .map(|n| {
                (
                    "アパレル".to_string(),
                    format!("・商品{n}（1000円）タグ：なし"),
                )
            })
            .collect();
        let prompt = build_system_prompt(&["アパレル".to_string()], &[], &digest, &[], &brief, 2);
        assert!(prompt.contains("商品1"));
        assert!(prompt.contains("商品2"));
        assert!(!prompt.contains("商品3"));
        assert!(prompt.contains("（ほか1点）"));
    }
}
