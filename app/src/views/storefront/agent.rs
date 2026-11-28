//! View types for the shopping-agent page.
//!
//! The chat page carries the conversation and, after a run,
//! the results. The per-pick explanations are built here from
//! the engine's match data under the locked wording
//! discipline: a tag match is stated as a fact, a keyword
//! match is only ever a mention in the description, and an
//! unrated product is shown as scored on the neutral prior —
//! never as if it had a rating.

use askama::Template;

use super::agent_settlement::AgentSettlementView;
use super::header::Header;
use super::product_card::{ProductCard, ProductCardAction};
use crate::services::agent_run_service::{AgentPick, MatchedVia};

/// One pick as the results list renders it: display strings
/// precomputed, so the template only lays out.
pub struct PickView {
    /// Product id (the canonical detail-page key).
    pub product_id: i64,
    /// Chosen variant id (the add-anyway form's target).
    pub variant_id: i64,
    /// Product name.
    pub name: String,
    /// Chosen variant's label (empty for a plain product).
    pub variant_label: String,
    /// Unit price, formatted.
    pub formatted_price: String,
    /// Weighted total score, as points out of 100.
    pub total_score: i64,
    /// Price factor score, as points out of 100.
    pub price_score: i64,
    /// Rating factor score, as points out of 100.
    pub rating_score: i64,
    /// Needs factor score, as points out of 100.
    pub needs_score: i64,
    /// Stock factor score, as points out of 100.
    pub stock_score: i64,
    /// Explanation lines, in display order.
    pub reasons: Vec<String>,
    /// The product's card, when its card data loaded: the
    /// results render the shared card partial, with the action
    /// set by which list the pick sits in.
    pub card: Option<ProductCard>,
}

impl PickView {
    /// Builds the display record for one engine pick, with
    /// its card (its action already set by the caller).
    fn from_pick(pick: &AgentPick, card: Option<ProductCard>) -> PickView {
        let mut reasons = Vec::new();
        for matched in &pick.need_matches {
            match matched.via {
                // A tag match is a stored fact about the
                // product, so it is stated as one.
                MatchedVia::Tag => {
                    reasons.push(format!("タグ「{}」に一致", matched.need));
                }
                // A keyword match proves only that the words
                // appear; the copy must never upgrade it to
                // a guaranteed property.
                MatchedVia::Keyword => {
                    reasons.push(format!("説明文に「{}」の記載があります", matched.need));
                }
                MatchedVia::None => {}
            }
        }
        if pick.rating_count > 0 {
            reasons.push(format!(
                "評価 ★{:.1}（{}件のレビュー）",
                pick.rating, pick.rating_count
            ));
        } else {
            reasons.push("未評価のため中間の評価で計算".to_string());
        }
        PickView {
            product_id: pick.product_id,
            variant_id: pick.variant_id,
            name: pick.name.clone(),
            variant_label: pick.variant_label.clone(),
            formatted_price: crate::pricing::display_price(pick.unit_price_yen),
            total_score: points(pick.scores.total),
            price_score: points(pick.scores.price),
            rating_score: points(pick.scores.rating),
            needs_score: points(pick.scores.needs),
            stock_score: points(pick.scores.stock),
            reasons,
            card,
        }
    }
}

/// A score in 0.0..=1.0 as points out of 100.
fn points(score: f64) -> i64 {
    (score * 100.0).round() as i64
}

/// One budget-refused pick as the over-budget list renders
/// it, with the exact amounts the refusal rests on.
pub struct OverBudgetView {
    /// The refused pick.
    pub pick: PickView,
    /// The cart total adding it would have produced,
    /// formatted.
    pub formatted_projected_total: String,
    /// The exact overage over the budget, formatted.
    pub formatted_overage: String,
}

/// One proposal's outcome as the results section renders it.
pub struct AgentResultView {
    /// Picks the proposal offers, best first. A pick is in
    /// the cart only after the shopper applied it.
    pub picks: Vec<PickView>,
    /// Picks the budget refused.
    pub over_budget: Vec<OverBudgetView>,
    /// Picks skipped because the owner already had the
    /// variant in the cart by hand.
    pub already_in_cart: Vec<PickView>,
    /// Must-haves no pick satisfied by tag (the run relaxed
    /// them); empty when every must-have was met.
    pub unmet_needs: Vec<String>,
    /// The run's budget, formatted （指定なし when the brief
    /// stated none).
    pub formatted_budget: String,
    /// Projected subtotal with the picks applied, formatted.
    pub formatted_subtotal: String,
    /// Projected shipping with the picks applied, formatted.
    pub formatted_shipping: String,
    /// Projected total with the picks applied, formatted.
    pub formatted_total: String,
    /// Budget minus total, formatted (never negative).
    pub formatted_remaining: String,
    /// Projected total with the picks applied, in yen.
    pub total_yen: i64,
    /// The proposal's budget, in yen, when it had one.
    pub budget_yen: Option<i64>,
    /// Whether the proposal enforced a budget at all.
    pub has_budget: bool,
    /// Whether the apply-all action renders under this
    /// result: only on the thread's latest proposal, and
    /// only while it still has unapplied picks.
    pub show_apply_all: bool,
}

impl AgentResultView {
    /// Builds the display record from a chat thread's
    /// snapshotted proposal: the same
    /// lists and projected totals, rendered identically to
    /// a fresh proposal.
    ///
    /// `applied` carries the variant ids of this proposal
    /// the shopper has already applied: their cards render
    /// the in-cart state, the rest render their per-pick
    /// apply button. `is_latest` marks the thread's newest
    /// proposal — the only one the apply-all action renders
    /// under, so a stale result in the history can never
    /// apply anything but its own picks one by one.
    pub fn from_snapshot(
        snapshot: &crate::services::agent_chat_service::ChatResultSnapshot,
        cards: Vec<ProductCard>,
        applied: &std::collections::HashSet<i64>,
        is_latest: bool,
    ) -> AgentResultView {
        Self::build(
            &snapshot.picks,
            &snapshot.over_budget,
            &snapshot.already_in_cart,
            snapshot.unmet_needs.clone(),
            snapshot.subtotal_yen,
            snapshot.shipping_yen,
            snapshot.total_yen,
            snapshot.budget_yen,
            cards,
            applied,
            is_latest,
        )
    }

    /// The builder behind [`Self::from_snapshot`]: one
    /// proposal's lists and projected totals.
    #[allow(clippy::too_many_arguments)]
    fn build(
        picks: &[crate::services::agent_run_service::AgentPick],
        over_budget: &[crate::services::agent_run_service::OverBudgetPick],
        already_in_cart: &[crate::services::agent_run_service::AgentPick],
        unmet_needs: Vec<String>,
        subtotal_yen: i64,
        shipping_yen: i64,
        total_yen: i64,
        budget_yen: Option<i64>,
        cards: Vec<ProductCard>,
        applied: &std::collections::HashSet<i64>,
        is_latest: bool,
    ) -> AgentResultView {
        let mut cards_by_id: std::collections::HashMap<i64, ProductCard> = cards
            .into_iter()
            .map(|card| (card.product.id, card))
            .collect();
        AgentResultView {
            picks: picks
                .iter()
                .map(|pick| {
                    let mut card = cards_by_id.remove(&pick.product_id);
                    if let Some(card) = card.as_mut() {
                        card.action = if applied.contains(&pick.variant_id) {
                            ProductCardAction::InCart
                        } else {
                            ProductCardAction::ApplyPick {
                                variant_id: pick.variant_id,
                            }
                        };
                    }
                    PickView::from_pick(pick, card)
                })
                .collect(),
            over_budget: over_budget
                .iter()
                .map(|entry| {
                    let mut card = cards_by_id.remove(&entry.pick.product_id);
                    if let Some(card) = card.as_mut() {
                        card.action = ProductCardAction::AddAnyway {
                            variant_id: entry.pick.variant_id,
                        };
                    }
                    OverBudgetView {
                        pick: PickView::from_pick(&entry.pick, card),
                        formatted_projected_total: crate::pricing::display_price(
                            entry.projected_total_yen,
                        ),
                        formatted_overage: crate::pricing::display_price(entry.overage_yen),
                    }
                })
                .collect(),
            already_in_cart: already_in_cart
                .iter()
                .map(|pick| PickView::from_pick(pick, None))
                .collect(),
            unmet_needs,
            formatted_budget: match budget_yen {
                Some(yen) => crate::pricing::display_price(yen),
                None => "指定なし".to_string(),
            },
            formatted_subtotal: crate::pricing::display_price(subtotal_yen),
            formatted_shipping: crate::pricing::display_price(shipping_yen),
            formatted_total: crate::pricing::display_price(total_yen),
            formatted_remaining: crate::pricing::display_price(
                budget_yen.map_or(0, |yen| (yen - total_yen).max(0)),
            ),
            total_yen,
            budget_yen,
            has_budget: budget_yen.is_some(),
            show_apply_all: is_latest
                && picks.iter().any(|pick| !applied.contains(&pick.variant_id)),
        }
    }

    /// Whether the run produced nothing to show in any list.
    pub fn is_empty(&self) -> bool {
        self.picks.is_empty() && self.over_budget.is_empty() && self.already_in_cart.is_empty()
    }

    /// Whether the cart the run left fits the budget (a run
    /// without a budget always fits).
    pub fn within_budget(&self) -> bool {
        self.budget_yen.is_none_or(|yen| self.total_yen <= yen)
    }

    /// Whether the totals block shows the "left of the
    /// budget" row: only a budgeted run that stayed inside
    /// its budget has anything left to state.
    pub fn show_remaining(&self) -> bool {
        self.has_budget && self.within_budget()
    }
}

/// One thread entry as the chat page renders it: a visitor
/// bubble, an agent bubble, or an agent search result (its
/// lead-in text plus the shared results block).
pub struct ChatEntryView {
    /// Whether this entry is the visitor's own message.
    pub is_user: bool,
    /// Whether this entry is a search result.
    pub is_result: bool,
    /// The bubble text (the lead-in message for a result).
    pub text: String,
    /// The run outcome, for a result entry.
    pub result: Option<AgentResultView>,
}

/// The chat face of the agent page:
/// the conversation thread, the preset chips and the input
/// bar. The thread renders from the session's stored entries;
/// an empty thread shows the greeting and the example chips.
#[derive(Template)]
#[template(path = "storefront/pages/agent_chat.html")]
pub struct AgentChatView {
    /// Header data.
    pub header: Header,
    /// The agent's greeting, shown on an empty thread.
    pub greeting: String,
    /// Example prompts offered as chips on an empty thread.
    pub examples: Vec<String>,
    /// Thread entries, oldest first.
    pub entries: Vec<ChatEntryView>,
    /// The preset currently in force (a `Preset` form value).
    pub preset: String,
    /// The accumulated brief as removable chips.
    pub brief_chips: Vec<BriefChip>,
    /// A one-off note (an empty message was refused).
    pub note: Option<String>,
    /// The agent settlement slot under the thread
    ///, when the cart holds agent lines.
    pub settlement: Option<AgentSettlementView>,
}

/// The htmx answer to a chat post:
/// only the entries the turn appended, plus two
/// out-of-band updates — the composer's note slot and the
/// header cart badge. The fields `count` and `oob` are the
/// context of the shared cart-badge part this includes.
#[derive(Template)]
#[template(path = "storefront/parts/agent_chat_fragment.html")]
pub struct AgentChatFragmentView {
    /// The entries this turn appended, oldest first.
    pub entries: Vec<ChatEntryView>,
    /// A one-off note for the composer slot (an empty
    /// message was refused); `None` clears the slot.
    pub note: Option<String>,
    /// Total quantity in the visitor's cart, for the badge.
    pub count: i64,
    /// The badge copy is always the out-of-band one here.
    pub oob: bool,
    /// The agent settlement slot, refreshed
    /// out-of-band with the turn that may have changed it.
    pub settlement: Option<AgentSettlementView>,
    /// The accumulated brief as removable chips, refreshed
    /// out-of-band with the turn that may have changed it.
    pub brief_chips: Vec<BriefChip>,
}

/// The preset chips alone: the chat page includes this
/// part, and an htmx chip tap is answered with it.
#[derive(Template)]
#[template(path = "storefront/parts/agent_presets.html")]
pub struct AgentPresetsView {
    /// The preset currently in force (a `Preset` form value).
    pub preset: String,
}

/// One accumulated brief condition as a removable chip on
/// the chat page: what the agent is currently honouring,
/// with the form values its clear button posts.
pub struct BriefChip {
    /// The chip's text (the condition, in shopper words).
    pub label: String,
    /// The brief slot the clear post names.
    pub field: String,
    /// The need text for a list slot; empty for single slots.
    pub value: String,
}

/// The brief chips for a conversation, in the order the
/// conditions read: budget, category, then must-haves and
/// nice-to-haves as stated. The preset is not a chip here —
/// it has its own chip row in the composer.
pub fn brief_chips(brief: &crate::models::agent::ChatBrief) -> Vec<BriefChip> {
    let mut chips = Vec::new();
    if let Some(budget) = brief.budget_yen {
        chips.push(BriefChip {
            label: format!("予算：{}", crate::pricing::display_price(budget)),
            field: "budget".to_string(),
            value: String::new(),
        });
    }
    if let Some(category) = &brief.category {
        chips.push(BriefChip {
            label: format!("カテゴリー：{category}"),
            field: "category".to_string(),
            value: category.clone(),
        });
    }
    for need in &brief.must_have {
        chips.push(BriefChip {
            label: format!("必須：{need}"),
            field: "must_have".to_string(),
            value: need.clone(),
        });
    }
    for need in &brief.nice_to_have {
        chips.push(BriefChip {
            label: format!("希望：{need}"),
            field: "nice_to_have".to_string(),
            value: need.clone(),
        });
    }
    chips
}
