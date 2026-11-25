//! Shopping-agent run engine, split by
//! capability under the design rule:
//! agent replies PROPOSE; cart writes require an explicit
//! shopper action.
//!
//! - [`propose`] is READ-ONLY. It assembles the public
//!   catalog through the catalog service, scores
//!   every product against the brief, selects picks and
//!   computes the budget fit and the projected totals. It
//!   has no path to a cart mutation, so an agent reply —
//!   the chat handler calls only this — can never change
//!   the cart, no matter what the model returns.
//! - [`apply`] is the engine's ONLY cart-writing
//!   operation, and it may be invoked only from a
//!   deliberate shopper action that carries the picks
//!   being applied: a per-pick 「カートに入れる」 click on
//!   a chat proposal, the proposal's 「すべてカートに入れる」
//!   action, or the over-budget add-anyway post (which
//!   predates the split and follows the same rule).
//!   Checkout still requires the separate human approval
//!   flow (Sessions 6/6.5).
//!
//! The budget is a hard cap on the projected cart total
//! including shipping: a pick that would cross it is
//! reported as over-budget with the exact overage, and is
//! never among the picks a shopper can apply from the
//! proposal (add-anyway is its deliberate exception).

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::CartLine;
use crate::models::agent::Brief;
use crate::repositories::cart_repository;
use crate::repositories::product_repository;
use crate::services::api_catalog_service;
use crate::services::cart_service::{self, AddOutcome, CartOwner, CartTotals};
use crate::services::tax_service::TaxParams;
use crate::util::rate_limit_knobs::SWEEP_THRESHOLD;

/// The rating an unrated product is scored with: the neutral
/// prior from the session's locked decisions. A product with
/// reviews is always scored on its real aggregate instead.
pub const UNRATED_PRIOR: f64 = 3.5;

/// How many phantom reviews at [`UNRATED_PRIOR`] the Bayesian
/// rating smoothing mixes into a product's real average: with
/// a weight of 10, one 5-star review scores about 3.6 rather
/// than 5.0, and a genuinely below-prior average scores below
/// the prior from the very first review — a rated product
/// never outranks an unrated one on a technicality, nor hides
/// behind the prior either.
const RATING_PRIOR_WEIGHT: f64 = 10.0;

/// Distinct products one run may pick (locked session cap).
const MAX_PICKS: usize = 3;

/// Needs credit for a tag match: the full need.
const TAG_CREDIT: f64 = 1.0;

/// Needs credit for a keyword match in the name or
/// description: partial only — a keyword supports a need but
/// can never satisfy a must-have on its own.
const KEYWORD_CREDIT: f64 = 0.5;

/// Negation cues: a keyword occurrence followed by one of
/// these in the same sentence segment does not count as a
/// mention (the locked MVP rule, deliberately narrow).
const NEGATION_CUES: [&str; 5] = ["ではありません", "ではない", "なし", "非対応", "対象外"];

/// One tag on a candidate product.
#[derive(Clone, Debug)]
pub struct CandidateTag {
    /// Stable ASCII identifier.
    pub slug: String,
    /// Japanese label shown to people.
    pub label: String,
}

/// One combination of a candidate product.
#[derive(Clone, Debug)]
pub struct CandidateVariant {
    /// Variant id (the cart line identity).
    pub id: i64,
    /// Combination label; empty for a plain product.
    pub label: String,
    /// The combination's price in integer yen.
    pub price_yen: i64,
    /// Units in stock for this combination.
    pub stock: i32,
    /// Whether the combination can be bought right now, under
    /// the catalog service's rule. For a variant with stock on
    /// hand this is exactly its enabled flag.
    pub purchasable: bool,
}

/// One product the agent considers, in the catalog service's
/// terms plus the description its keyword matching reads.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// Product id.
    pub product_id: i64,
    /// Product name.
    pub name: String,
    /// Product description (keyword-matching source).
    pub description: String,
    /// Effective price in integer yen, tax included.
    pub price_yen: i64,
    /// Rating used for scoring: the live reviews average, or
    /// [`UNRATED_PRIOR`] when the product has no reviews.
    pub rating: f64,
    /// Number of reviews behind the average (0 when unrated).
    pub rating_count: i64,
    /// Category name (empty when the product has none).
    pub category_name: String,
    /// The product's tags.
    pub tags: Vec<CandidateTag>,
    /// Stock summed over the product's enabled combinations.
    pub total_stock: i32,
    /// The product's combinations.
    pub variants: Vec<CandidateVariant>,
}

/// How one need matched one product.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MatchedVia {
    /// Matched a product tag (full credit; satisfies a
    /// must-have).
    Tag,
    /// Matched a keyword in the name or description (partial
    /// credit; never satisfies a must-have).
    Keyword,
    /// Did not match.
    None,
}

/// The outcome of matching one brief need against one product.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NeedMatch {
    /// The need, as written in the brief.
    pub need: String,
    /// Whether the need came from the must-have list.
    pub must_have: bool,
    /// Credit earned: 1.0 by tag, 0.5 by keyword, 0.0 on a miss.
    pub credit: f64,
    /// How the need matched.
    pub via: MatchedVia,
}

/// One candidate's factor scores, each in 0.0..=1.0, and the
/// weighted total under the brief's preset.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct FactorScores {
    /// Pool-normalised price score (cheapest in the pool: 1.0).
    pub price: f64,
    /// Rating score: the rating used, over 5.
    pub rating: f64,
    /// Needs score: the mean credit over the brief's needs
    /// (1.0 when the brief states no needs).
    pub needs: f64,
    /// Stock score: total stock, capped at 10 units, over 10.
    pub stock: f64,
    /// Weighted total under the brief's preset.
    pub total: f64,
}

/// A candidate with its scores and the variant a run would
/// pick for it (`None` when no combination is in stock and
/// enabled, so the product cannot be picked at all).
#[derive(Clone, Debug)]
pub struct ScoredCandidate {
    /// The candidate product.
    pub candidate: Candidate,
    /// Its factor scores.
    pub scores: FactorScores,
    /// Per-need match detail, in brief order.
    pub need_matches: Vec<NeedMatch>,
    /// How many must-haves this product satisfies (by tag).
    pub must_haves_met: usize,
    /// The combination a run would add, when pickable.
    pub chosen_variant: Option<CandidateVariant>,
}

/// One product a run picked (or would have picked), with the
/// data its explanation renders from.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AgentPick {
    /// Product id.
    pub product_id: i64,
    /// Product name.
    pub name: String,
    /// The chosen combination's variant id.
    pub variant_id: i64,
    /// The chosen combination's label (shown in the
    /// explanation, per the locked variant-choice rule).
    pub variant_label: String,
    /// Unit price of the chosen combination, in yen.
    pub unit_price_yen: i64,
    /// The product's factor scores.
    pub scores: FactorScores,
    /// Per-need match detail.
    pub need_matches: Vec<NeedMatch>,
    /// The rating the score used (aggregate or prior).
    pub rating: f64,
    /// Reviews behind the rating (0 when the prior was used).
    pub rating_count: i64,
}

/// A pick the budget refused, with the exact amount by which
/// adding it would have crossed the budget.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct OverBudgetPick {
    /// The refused pick.
    pub pick: AgentPick,
    /// The cart total that adding it would have produced.
    pub projected_total_yen: i64,
    /// `projected_total_yen` minus the budget.
    pub overage_yen: i64,
}

/// The structured outcome of one agent proposal: what the
/// engine decided, with NO cart writes behind it ([`propose`]).
#[derive(Clone, Debug)]
pub struct AgentProposal {
    /// Picks the shopper may apply, best first.
    pub picks: Vec<AgentPick>,
    /// Picks the budget refused, in score order.
    pub over_budget: Vec<OverBudgetPick>,
    /// Picks skipped because the variant was already in the
    /// cart as the owner's own line (left untouched).
    pub already_in_cart: Vec<AgentPick>,
    /// Must-haves no eligible product satisfied. Non-empty
    /// only when the must-have rule had to be relaxed.
    pub unmet_needs: Vec<String>,
    /// Projected totals with the picks applied — the owner's
    /// own lines plus the picks, through the cart service's
    /// one totals computation. The owner's previous agent
    /// lines count for nothing in the projection: applying a
    /// proposal replaces them.
    pub projected_totals: CartTotals,
}

/// The outcome of writing picks into the cart ([`apply`]).
#[derive(Clone, Debug)]
pub struct AgentApplyOutcome {
    /// Picks that actually landed as agent lines, in the
    /// order given. A pick whose product changed since the
    /// proposal (sold out, taken off sale) honestly drops
    /// out instead of failing the whole apply.
    pub landed: Vec<AgentPick>,
    /// Totals of the cart as the apply left it.
    pub totals: CartTotals,
}

/// The rating a product is scored with: its live average when
/// it has reviews, the neutral prior when it has none.
fn rating_or_prior(average: Option<f64>, count: i64) -> f64 {
    match average {
        Some(value) if count > 0 => value,
        _ => UNRATED_PRIOR,
    }
}

/// The rating the score factor uses: the product's average
/// pulled toward [`UNRATED_PRIOR`] by [`RATING_PRIOR_WEIGHT`]
/// phantom reviews (Bayesian smoothing), so a thin review
/// history cannot dominate the ranking. The rating shown to
/// the shopper stays the raw average — only the score is
/// smoothed.
fn smoothed_rating(rating: f64, count: i64) -> f64 {
    if count <= 0 {
        return UNRATED_PRIOR;
    }
    let n = count as f64;
    (n * rating + RATING_PRIOR_WEIGHT * UNRATED_PRIOR) / (n + RATING_PRIOR_WEIGHT)
}

/// Assembles the candidate pool from the catalog service:
/// every public product (all pages), each with its
/// combinations from the detail read.
async fn assemble_candidates(pool: &PgPool) -> Result<Vec<Candidate>, AppError> {
    let first = api_catalog_service::list_products(pool, 1).await?;
    let mut products = first.products;
    for page in 2..=first.total_pages {
        products.extend(
            api_catalog_service::list_products(pool, page)
                .await?
                .products,
        );
    }
    // The combinations arrive with one bulk load:
    //  the per-product detail read this loop ran
    // before cost about six queries per product.
    let ids: Vec<i64> = products.iter().map(|p| p.id).collect();
    let rows = product_repository::find_by_ids(pool, &ids).await?;
    let mut variants_by_product = api_catalog_service::variants_by_product(pool, &rows).await?;
    let mut candidates = Vec::with_capacity(products.len());
    for product in products {
        let variants = variants_by_product
            .remove(&product.id)
            .unwrap_or_default()
            .into_iter()
            .map(|v| CandidateVariant {
                id: v.id,
                label: v.label,
                price_yen: v.price_yen,
                stock: v.stock,
                purchasable: v.purchasable,
            })
            .collect();
        candidates.push(Candidate {
            product_id: product.id,
            name: product.name,
            description: product.description,
            price_yen: product.price_yen,
            rating: rating_or_prior(product.rating.average, product.rating.count),
            rating_count: product.rating.count,
            category_name: product.category.name,
            tags: product
                .tags
                .into_iter()
                .map(|t| CandidateTag {
                    slug: t.slug,
                    label: t.label,
                })
                .collect(),
            total_stock: product.stock,
            variants,
        });
    }
    // The card reads behind the catalog service never fetch
    // the long description, so candidates arrive with an
    // empty one and keyword matching would only ever see
    // product names. Fill the real texts in with one batch
    // query over the assembled pool.
    let ids: Vec<i64> = candidates.iter().map(|c| c.product_id).collect();
    let descriptions = product_repository::descriptions_for_products(pool, &ids).await?;
    for candidate in &mut candidates {
        if let Some(description) = descriptions.get(&candidate.product_id) {
            candidate.description = description.clone();
        }
    }
    Ok(candidates)
}

/// Matches one need against one product: a tag match earns
/// full credit, a keyword match in the name or description
/// partial credit, a miss none.
fn match_need(candidate: &Candidate, need: &str) -> (f64, MatchedVia) {
    let by_tag = candidate
        .tags
        .iter()
        .any(|tag| tag.label.contains(need) || tag.slug.eq_ignore_ascii_case(need));
    if by_tag {
        return (TAG_CREDIT, MatchedVia::Tag);
    }
    let text = format!("{}\n{}", candidate.name, candidate.description);
    if keyword_matches(&text, need) {
        return (KEYWORD_CREDIT, MatchedVia::Keyword);
    }
    (0.0, MatchedVia::None)
}

/// Whether `keyword` is mentioned in `text` at least once
/// without being negated. An occurrence is negated when a
/// negation cue follows it within the same sentence segment
/// (segments split on 。！？ and newlines) — so
/// 「防水ではありません」 never counts as a mention of 防水,
/// while a separate positive mention in another segment does.
fn keyword_matches(text: &str, keyword: &str) -> bool {
    if keyword.is_empty() {
        return false;
    }
    for segment in text.split(['。', '！', '？', '\n']) {
        let mut rest = segment;
        while let Some(at) = rest.find(keyword) {
            let after = &rest[at + keyword.len()..];
            if !NEGATION_CUES.iter().any(|cue| after.contains(cue)) {
                return true;
            }
            rest = after;
        }
    }
    false
}

/// Chooses the combination a run would add for a product:
/// when a brief need matches a combination's label, the
/// cheapest in-stock combination among the label matches;
/// otherwise the combination with the most stock in hand
/// (ties: cheapest, then lowest id) — the variant the shop
/// itself stocked deepest, and the one least likely to sell
/// out from under the shopper. `None` when nothing is in
/// stock and enabled.
fn choose_variant(candidate: &Candidate, brief: &Brief) -> Option<CandidateVariant> {
    let in_stock: Vec<&CandidateVariant> = candidate
        .variants
        .iter()
        .filter(|v| v.stock > 0 && v.purchasable)
        .collect();
    if in_stock.is_empty() {
        return None;
    }
    let labelled: Vec<&CandidateVariant> = in_stock
        .iter()
        .copied()
        .filter(|v| {
            brief
                .all_needs()
                .any(|need| v.label.contains(need.as_str()))
        })
        .collect();
    if labelled.is_empty() {
        in_stock
            .iter()
            .min_by(|a, b| {
                b.stock
                    .cmp(&a.stock)
                    .then_with(|| a.price_yen.cmp(&b.price_yen))
                    .then_with(|| a.id.cmp(&b.id))
            })
            .map(|variant| (*variant).clone())
    } else {
        labelled
            .iter()
            .min_by(|a, b| a.price_yen.cmp(&b.price_yen).then_with(|| a.id.cmp(&b.id)))
            .map(|variant| (*variant).clone())
    }
}

/// Scores every candidate against the brief. Price is
/// normalised across this pool: cheapest 1.0, priciest 0.0,
/// and 1.0 for all when every price is equal.
fn score_candidates(candidates: &[Candidate], brief: &Brief) -> Vec<ScoredCandidate> {
    let weights = brief.preset.weights();
    let pool_min = candidates.iter().map(|c| c.price_yen).min().unwrap_or(0);
    let pool_max = candidates.iter().map(|c| c.price_yen).max().unwrap_or(0);
    candidates
        .iter()
        .map(|candidate| {
            let price = if pool_max > pool_min {
                (pool_max - candidate.price_yen) as f64 / (pool_max - pool_min) as f64
            } else {
                1.0
            };
            let need_matches: Vec<NeedMatch> = brief
                .must_have
                .iter()
                .map(|need| (need, true))
                .chain(brief.nice_to_have.iter().map(|need| (need, false)))
                .map(|(need, must_have)| {
                    let (credit, via) = match_need(candidate, need);
                    NeedMatch {
                        need: need.clone(),
                        must_have,
                        credit,
                        via,
                    }
                })
                .collect();
            let needs = if need_matches.is_empty() {
                1.0
            } else {
                need_matches.iter().map(|m| m.credit).sum::<f64>() / need_matches.len() as f64
            };
            let must_haves_met = need_matches
                .iter()
                .filter(|m| m.must_have && m.via == MatchedVia::Tag)
                .count();
            let scores = FactorScores {
                price,
                rating: smoothed_rating(candidate.rating, candidate.rating_count) / 5.0,
                needs,
                stock: f64::from(candidate.total_stock.min(10)) / 10.0,
                total: 0.0,
            };
            let total = weights.price * scores.price
                + weights.rating * scores.rating
                + weights.needs * scores.needs
                + weights.stock * scores.stock;
            ScoredCandidate {
                candidate: candidate.clone(),
                scores: FactorScores { total, ..scores },
                must_haves_met,
                chosen_variant: choose_variant(candidate, brief),
                need_matches,
            }
        })
        .collect()
}

/// Applies the must-have rule over the pickable candidates:
/// those satisfying every must-have are eligible; when none
/// do, eligibility relaxes to those satisfying the most
/// must-haves, and the must-haves no eligible candidate
/// satisfies are returned as unmet.
/// Whether any candidate in the scored pool matched any of
/// the brief's needs at all — by tag or by keyword. When a
/// brief carries needs and this is false, the search found
/// nothing relevant, and ranking the pool anyway would
/// degenerate to price order (every needs score is zero),
/// presenting the cheapest products as if they were
/// recommendations.
fn pool_has_need_match(scored: &[ScoredCandidate]) -> bool {
    scored
        .iter()
        .any(|s| s.need_matches.iter().any(|m| m.credit > 0.0))
}

fn eligible_pool(scored: &[ScoredCandidate], brief: &Brief) -> (Vec<usize>, Vec<String>) {
    let pickable: Vec<usize> = scored
        .iter()
        .enumerate()
        .filter(|(_, s)| s.chosen_variant.is_some())
        .map(|(i, _)| i)
        .collect();
    if brief.must_have.is_empty() {
        return (pickable, Vec::new());
    }
    let max_met = pickable
        .iter()
        .map(|&i| scored[i].must_haves_met)
        .max()
        .unwrap_or(0);
    let eligible: Vec<usize> = pickable
        .into_iter()
        .filter(|&i| scored[i].must_haves_met == max_met)
        .collect();
    let unmet = brief
        .must_have
        .iter()
        .filter(|need| {
            !eligible.iter().any(|&i| {
                scored[i]
                    .need_matches
                    .iter()
                    .any(|m| m.must_have && &m.need == *need && m.via == MatchedVia::Tag)
            })
        })
        .cloned()
        .collect();
    (eligible, unmet)
}

/// Projects the cart total with `candidate` added to the
/// owner's own lines plus the picks so far, through the cart
/// service's one totals computation. `Ok` carries the totals
/// when the projection fits the budget; `Err` carries the
/// projected total and the exact overage in yen. A brief
/// without a budget (the chat flow's unstated state) fits
/// everything: no cap, nothing over-budget.
fn try_fit(
    base: &[CartLine],
    picked: &[CartLine],
    candidate: &CartLine,
    budget_yen: Option<i64>,
    tax: TaxParams,
    fees: crate::models::FeeSettings,
) -> Result<CartTotals, (i64, i64)> {
    let mut lines: Vec<CartLine> = base.to_vec();
    lines.extend(picked.iter().cloned());
    lines.push(candidate.clone());
    let totals = cart_service::totals(&lines, 0, 0, None, tax, fees);
    match budget_yen {
        Some(budget) if totals.total_yen > budget => {
            Err((totals.total_yen, totals.total_yen - budget))
        }
        _ => Ok(totals),
    }
}

/// Builds the explanation record for one scored candidate.
fn pick_of(scored: &ScoredCandidate) -> AgentPick {
    let variant = scored
        .chosen_variant
        .as_ref()
        .expect("pick records are built only for pickable candidates");
    AgentPick {
        product_id: scored.candidate.product_id,
        name: scored.candidate.name.clone(),
        variant_id: variant.id,
        variant_label: variant.label.clone(),
        unit_price_yen: variant.price_yen,
        scores: scored.scores,
        need_matches: scored.need_matches.clone(),
        rating: scored.candidate.rating,
        rating_count: scored.candidate.rating_count,
    }
}

/// Computes one proposal for one cart owner against one
/// brief — READ-ONLY by construction: it reads the owner's
/// lines for the projection base and never writes to the
/// cart. This is the only engine operation the chat handler
/// may call, so a chat reply can never mutate the cart.
pub async fn propose(
    pool: &PgPool,
    owner: &CartOwner,
    brief: &Brief,
) -> Result<AgentProposal, AppError> {
    let tax = crate::services::tax_service::params_today_or_fallback(pool).await;

    // The pool is the brief's category when one was picked;
    // scoring and price normalisation see only that pool.
    let mut candidates = assemble_candidates(pool).await?;
    if let Some(category) = &brief.category {
        candidates.retain(|c| &c.category_name == category);
    }
    let scored = score_candidates(&candidates, brief);
    let (eligible, unmet_needs) = eligible_pool(&scored, brief);
    let mut ranked: Vec<usize> = eligible;
    ranked.sort_by(|&a, &b| {
        scored[b]
            .scores
            .total
            .partial_cmp(&scored[a].scores.total)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                scored[a]
                    .candidate
                    .price_yen
                    .cmp(&scored[b].candidate.price_yen)
            })
            .then_with(|| {
                scored[a]
                    .candidate
                    .product_id
                    .cmp(&scored[b].candidate.product_id)
            })
    });

    // The owner's own lines are the projection base. Their
    // previous agent lines count for nothing here: applying
    // this proposal would replace them, so the projected
    // totals are the owner's own lines plus these picks.
    let current = cart_service::lines(pool, owner).await?;
    let human_lines: Vec<CartLine> = current
        .into_iter()
        .filter(|line| !line.added_by_agent)
        .collect();
    let human_variants: HashSet<i64> = human_lines.iter().map(|line| line.variant_id).collect();
    let fees = crate::services::shop_settings_service::load(pool)
        .await
        .fee_settings();

    // A brief carrying needs that nothing in the pool
    // matched — no tag, not even a keyword — has no honest
    // picks: return the empty proposal and let the surface
    // say the conditions matched nothing, instead of
    // dressing the pool's cheapest products up as
    // recommendations.
    let has_needs = !brief.must_have.is_empty() || !brief.nice_to_have.is_empty();
    if has_needs && !pool_has_need_match(&scored) {
        let projected_totals = cart_service::totals(&human_lines, 0, 0, None, tax, fees);
        tracing::info!("agent proposal completed: no candidate matched any need");
        return Ok(AgentProposal {
            picks: Vec::new(),
            over_budget: Vec::new(),
            already_in_cart: Vec::new(),
            unmet_needs,
            projected_totals,
        });
    }

    let mut picks: Vec<(AgentPick, CartLine)> = Vec::new();
    let mut over_budget: Vec<OverBudgetPick> = Vec::new();
    let mut already_in_cart: Vec<AgentPick> = Vec::new();
    for &index in &ranked {
        if picks.len() >= MAX_PICKS {
            break;
        }
        let entry = &scored[index];
        let variant = entry
            .chosen_variant
            .as_ref()
            .expect("eligible candidates are pickable");
        if human_variants.contains(&variant.id) {
            already_in_cart.push(pick_of(entry));
            continue;
        }
        let Some(preview) = cart_repository::line_for_variant(pool, variant.id, 1).await? else {
            continue;
        };
        let picked_lines: Vec<CartLine> = picks.iter().map(|(_, line)| line.clone()).collect();
        match try_fit(
            &human_lines,
            &picked_lines,
            &preview,
            brief.budget_yen,
            tax,
            fees,
        ) {
            Ok(_) => picks.push((pick_of(entry), preview)),
            Err((projected_total_yen, overage_yen)) => over_budget.push(OverBudgetPick {
                projected_total_yen,
                overage_yen,
                pick: pick_of(entry),
            }),
        }
    }

    let mut projected_lines = human_lines;
    projected_lines.extend(picks.iter().map(|(_, line)| line.clone()));
    let projected_totals = cart_service::totals(&projected_lines, 0, 0, None, tax, fees);
    tracing::info!(
        picks = picks.len(),
        over_budget = over_budget.len(),
        already_in_cart = already_in_cart.len(),
        "agent proposal completed"
    );
    Ok(AgentProposal {
        picks: picks.into_iter().map(|(pick, _)| pick).collect(),
        over_budget,
        already_in_cart,
        unmet_needs,
        projected_totals,
    })
}

/// Applies a proposal to the owner's cart under a per-owner
/// advisory lock — the public entry; the work itself is
/// in the private `apply_locked` function below.
///
/// The apply is a sequence of statements (delete the previous
/// agent lines, then upsert each pick), so two applies for
/// the same owner running at the same moment interleave:
/// both delete, then both insert, and the cart ends up with
/// the proposal's quantities doubled. The lock — held on one pooled connection for the
/// whole apply, the low-stock alert pass's pattern —
/// serialises applies per owner: the second waits, then
/// replaces the first's lines with the same picks, so any
/// number of concurrent applies lands the proposal exactly
/// once.
pub async fn apply(
    pool: &PgPool,
    owner: &CartOwner,
    picks: &[AgentPick],
    replace_previous: bool,
) -> Result<AgentApplyOutcome, AppError> {
    let lock_key = apply_lock_key(owner);
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| AppError::db("agent apply lock connection", e))?;
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *conn)
        .await
        .map_err(|e| AppError::db("agent apply lock", e))?;
    let result = apply_locked(pool, owner, picks, replace_previous).await;
    // The unlock must not fail silently: the lock is
    // session-scoped, so a connection returned to the pool
    // still holding it would keep this owner's applies
    // serialised — and stack re-entrant lock counts — for
    // that connection's whole life. On an unlock failure
    // the connection is closed instead of pooled.
    let unlocked = sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *conn)
        .await;
    if unlocked.is_err() {
        let _ = conn.close().await;
    }
    result
}

/// The advisory-lock key naming one cart owner's apply: the
/// two owner kinds get distinct namespaces, and
/// `hashtextextended` in SQL turns the text into the lock's
/// bigint key, so no Rust-side hash has to stay stable.
fn apply_lock_key(owner: &CartOwner) -> String {
    match owner {
        CartOwner::User(id) => format!("agent-apply:user:{id}"),
        CartOwner::Session(token) => format!("agent-apply:session:{token}"),
    }
}

/// Writes `picks` into the owner's cart as agent lines —
/// the engine's ONLY cart mutation. Callers are deliberate
/// shopper actions only (the module doc lists them); the
/// chat handler never calls this.
///
/// With `replace_previous`, the owner's previous agent
/// lines are deleted first — the apply-all merge rule: the
/// applied proposal supersedes the earlier one. Without
/// it, the picks land alongside the existing lines (a
/// per-pick apply). An empty pick list changes nothing
/// either way: applying an empty proposal never wipes
/// earlier lines.
async fn apply_locked(
    pool: &PgPool,
    owner: &CartOwner,
    picks: &[AgentPick],
    replace_previous: bool,
) -> Result<AgentApplyOutcome, AppError> {
    let tax = crate::services::tax_service::params_today_or_fallback(pool).await;
    let fees = crate::services::shop_settings_service::load(pool)
        .await
        .fee_settings();
    if picks.is_empty() {
        let lines = cart_service::lines(pool, owner).await?;
        return Ok(AgentApplyOutcome {
            landed: Vec::new(),
            totals: cart_service::totals(&lines, 0, 0, None, tax, fees),
        });
    }
    if replace_previous {
        cart_service::delete_agent_lines(pool, owner).await?;
    }
    let mut landed: Vec<AgentPick> = Vec::new();
    for pick in picks {
        match cart_service::add_to_cart_as_agent(pool, owner, pick.product_id, pick.variant_id, 1)
            .await?
        {
            AddOutcome::Added(_) | AddOutcome::Capped(_) => landed.push(pick.clone()),
            // The product changed between proposal and apply
            // (sold out or taken off sale): the pick honestly
            // drops out of the outcome.
            AddOutcome::SoldOut | AddOutcome::NotFound => {
                tracing::warn!(
                    product_id = pick.product_id,
                    variant_id = pick.variant_id,
                    "agent pick could not be added at apply time"
                );
            }
        }
    }
    let lines = cart_service::lines(pool, owner).await?;
    tracing::info!(landed = landed.len(), "agent proposal applied");
    Ok(AgentApplyOutcome {
        landed,
        totals: cart_service::totals(&lines, 0, 0, None, tax, fees),
    })
}

/// Runs one session may start inside the window (locked
/// session cap).
const RUN_LIMIT: usize = 10;

/// The run window: one hour, sliding.
const RUN_WINDOW: Duration = Duration::from_secs(60 * 60);

/// The message shown when a session reaches the run cap.
pub const RUN_LIMIT_MESSAGE: &str = "エージェントの実行回数の上限（1時間に10回）に達しました。しばらく時間をおいてから再度お試しください。";

/// Run start times per session key. In-memory and disposable
/// like the login throttle — a restart resets the counts,
/// which is accepted for the same reason: the cap is a
/// fairness guard against runaway loops, not a security
/// boundary.
static RUN_LOG: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Records a run start for `key` and returns whether it may
/// proceed: fewer than `RUN_LIMIT` starts inside the last
/// hour. A refused start is not recorded, so waiting out the
/// window always frees the session again.
pub fn try_begin_run(key: &str) -> bool {
    let now = Instant::now();
    // A poisoned mutex must not take the agent down with it:
    // recover the guard the way the login throttle does.
    let mut log = RUN_LOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if log.len() > SWEEP_THRESHOLD {
        log.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < RUN_WINDOW);
            !times.is_empty()
        });
    }
    let times = log.entry(key.to_string()).or_default();
    times.retain(|t| now.duration_since(*t) < RUN_WINDOW);
    if times.len() >= RUN_LIMIT {
        return false;
    }
    times.push(now);
    true
}

#[cfg(test)]
mod tests {
    use super::{
        Candidate, CandidateTag, CandidateVariant, MatchedVia, UNRATED_PRIOR, eligible_pool,
        keyword_matches, match_need, pool_has_need_match, rating_or_prior, score_candidates,
        smoothed_rating, try_fit,
    };
    use crate::models::CartLine;
    use crate::models::agent::{Brief, Preset};
    use crate::services::tax_service::TaxParams;

    fn candidate(id: i64, price: i64) -> Candidate {
        Candidate {
            product_id: id,
            name: format!("商品{id}"),
            description: String::new(),
            price_yen: price,
            rating: UNRATED_PRIOR,
            rating_count: 0,
            category_name: String::new(),
            tags: Vec::new(),
            total_stock: 5,
            variants: vec![CandidateVariant {
                id: id * 100,
                label: String::new(),
                price_yen: price,
                stock: 5,
                purchasable: true,
            }],
        }
    }

    fn brief() -> Brief {
        Brief {
            budget_yen: Some(100_000),
            category: None,
            must_have: Vec::new(),
            nice_to_have: Vec::new(),
            preset: Preset::Balanced,
        }
    }

    fn line(price: i64, stock: i32) -> CartLine {
        CartLine {
            variant_id: 1,
            product_id: 1,
            name: "テスト".to_string(),
            variant_label: String::new(),
            unit_price_yen: price,
            stock,
            image_path: String::new(),
            status: "public".to_string(),
            quantity: 1,
            added_by_agent: false,
            variant_enabled: true,
            publish_open: true,
            track_stock: true,
            allow_backorder: false,
            free_shipping: false,
            tax_class_code: "standard".to_string(),
            tax_class_name: "標準税率".to_string(),
            tax_rate_percent: 10,
            undeliverable_regions: String::new(),
        }
    }

    #[test]
    fn run_cap_refuses_the_eleventh_start_inside_an_hour() {
        let key = "unit-test-run-cap";
        for _ in 0..super::RUN_LIMIT {
            assert!(super::try_begin_run(key));
        }
        assert!(!super::try_begin_run(key));
        assert!(super::try_begin_run("unit-test-run-cap-other-session"));
    }

    #[test]
    fn unrated_products_score_on_the_prior() {
        assert_eq!(rating_or_prior(None, 0), 3.5);
        assert_eq!(rating_or_prior(Some(4.0), 0), 3.5);
        assert_eq!(rating_or_prior(Some(4.0), 12), 4.0);
        let scored = score_candidates(&[candidate(1, 1_000)], &brief());
        assert!((scored[0].scores.rating - 0.7).abs() < 1e-9);
    }

    #[test]
    fn price_score_normalises_across_the_pool() {
        let scored = score_candidates(&[candidate(1, 1_000), candidate(2, 3_000)], &brief());
        assert!((scored[0].scores.price - 1.0).abs() < 1e-9);
        assert!((scored[1].scores.price - 0.0).abs() < 1e-9);
        let equal = score_candidates(&[candidate(1, 2_000), candidate(2, 2_000)], &brief());
        assert!(equal.iter().all(|s| (s.scores.price - 1.0).abs() < 1e-9));
    }

    #[test]
    fn tag_match_earns_full_credit_keyword_only_partial() {
        let mut tagged = candidate(1, 1_000);
        tagged.tags = vec![CandidateTag {
            slug: "waterproof".to_string(),
            label: "防水".to_string(),
        }];
        let (credit, via) = match_need(&tagged, "防水");
        assert_eq!(credit, 1.0);
        assert_eq!(via, MatchedVia::Tag);

        let mut described = candidate(2, 1_000);
        described.description = "雨の日も安心の防水仕様です".to_string();
        let (credit, via) = match_need(&described, "防水");
        assert_eq!(credit, 0.5);
        assert_eq!(via, MatchedVia::Keyword);
    }

    #[test]
    fn keyword_match_never_satisfies_a_must_have() {
        let mut described = candidate(1, 1_000);
        described.description = "雨の日も安心の防水仕様です".to_string();
        let mut b = brief();
        b.must_have = vec!["防水".to_string()];
        let scored = score_candidates(&[described], &b);
        assert_eq!(scored[0].must_haves_met, 0);
        let (eligible, unmet) = eligible_pool(&scored, &b);
        assert_eq!(eligible, vec![0]);
        assert_eq!(unmet, vec!["防水"]);
    }

    #[test]
    fn pool_match_requires_some_need_credit() {
        // Nothing in the pool matches the need: no honest
        // recommendation exists.
        let mut b = brief();
        b.must_have = vec!["防水".to_string()];
        let scored = score_candidates(&[candidate(1, 1_000), candidate(2, 2_000)], &b);
        assert!(!pool_has_need_match(&scored));

        // A keyword mention anywhere in the pool is a match.
        let mut described = candidate(3, 3_000);
        described.description = "雨の日も安心の防水仕様です".to_string();
        let scored = score_candidates(&[described], &b);
        assert!(pool_has_need_match(&scored));

        // A brief without needs produces no need rows at all;
        // propose consults this helper only when the brief
        // carries needs.
        let scored = score_candidates(&[candidate(1, 1_000)], &brief());
        assert!(!pool_has_need_match(&scored));
    }

    #[test]
    fn negated_keyword_does_not_match_but_a_positive_mention_does() {
        // The locked mandatory case: 「防水ではありません」
        // must not match 防水.
        let mut negated = candidate(1, 1_000);
        negated.description = "この商品は防水ではありません。".to_string();
        let (credit, via) = match_need(&negated, "防水");
        assert_eq!(credit, 0.0);
        assert_eq!(via, MatchedVia::None);

        // A positive mention in its own segment still matches.
        assert!(keyword_matches(
            "防水ではありません。ただし防水ケースが付属します。",
            "防水"
        ));
        assert!(!keyword_matches("撥水加工のみで、防水なしです。", "防水"));
    }

    #[test]
    fn variant_choice_defaults_to_most_stock_unless_a_need_names_a_label() {
        let mut product = candidate(1, 2_000);
        product.variants = vec![
            CandidateVariant {
                id: 11,
                label: "カラー: ブラック".to_string(),
                price_yen: 1_500,
                stock: 3,
                purchasable: true,
            },
            CandidateVariant {
                id: 12,
                label: "カラー: ホワイト".to_string(),
                price_yen: 2_000,
                stock: 9,
                purchasable: true,
            },
            CandidateVariant {
                id: 13,
                label: "カラー: レッド".to_string(),
                price_yen: 1_000,
                stock: 0,
                purchasable: false,
            },
            CandidateVariant {
                id: 14,
                label: "カラー: ブラック（限定）".to_string(),
                price_yen: 1_800,
                stock: 7,
                purchasable: true,
            },
        ];
        // No need names a label: the deepest stock wins (12
        // holds 9), not the cheapest (11 at 1,500 yen).
        let scored = score_candidates(&[product.clone()], &brief());
        assert_eq!(scored[0].chosen_variant.as_ref().map(|v| v.id), Some(12));

        // A need naming ブラック narrows the pool to 11 and
        // 14; among label matches the cheapest still wins
        // (11), not the deepest stock (14 holds 7).
        let mut b = brief();
        b.nice_to_have = vec!["ブラック".to_string()];
        let scored = score_candidates(&[product], &b);
        assert_eq!(scored[0].chosen_variant.as_ref().map(|v| v.id), Some(11));
    }

    #[test]
    fn rating_score_is_smoothed_toward_the_prior() {
        // No reviews: exactly the prior.
        assert_eq!(smoothed_rating(UNRATED_PRIOR, 0), UNRATED_PRIOR);
        // One 5-star review: pulled most of the way down.
        assert!((smoothed_rating(5.0, 1) - 40.0 / 11.0).abs() < 1e-9);
        // A below-prior average scores below the prior from
        // the first reviews, while staying above its raw value.
        assert!(smoothed_rating(3.4, 3) < UNRATED_PRIOR);
        assert!(smoothed_rating(3.4, 3) > 3.4);
        // A deep history is barely moved.
        assert!((smoothed_rating(4.8, 200) - 995.0 / 210.0).abs() < 1e-9);
    }

    #[test]
    fn budget_cap_is_hard_and_the_overage_is_exact() {
        // One 2,980-yen pick in an empty cart: 2,980 + 550
        // shipping = 3,530. A 3,000 budget refuses it by 530.
        let pick = line(2_980, 10);
        let (projected, overage) = try_fit(
            &[],
            &[],
            &pick,
            Some(3_000),
            TaxParams::fallback(),
            crate::models::FeeSettings::fallback(),
        )
        .unwrap_err();
        assert_eq!(projected, 3_530);
        assert_eq!(overage, 530);
        let totals = try_fit(
            &[],
            &[],
            &pick,
            Some(3_530),
            TaxParams::fallback(),
            crate::models::FeeSettings::fallback(),
        )
        .unwrap();
        assert_eq!(totals.total_yen, 3_530);
    }

    #[test]
    fn a_brief_without_a_budget_fits_everything() {
        // The chat flow's unstated budget: no cap at all, so
        // even a projection past any plausible budget is a fit.
        let pick = line(2_980, 10);
        let totals = try_fit(
            &[],
            &[],
            &pick,
            None,
            TaxParams::fallback(),
            crate::models::FeeSettings::fallback(),
        )
        .unwrap();
        assert_eq!(totals.total_yen, 3_530);
    }

    #[test]
    fn budget_projection_counts_the_owners_existing_lines() {
        // The owner's own 5,000-yen line already earns free
        // shipping; a 550-yen pick projects to 5,550, not to
        // 550 + 550 shipping of its own.
        let own = line(5_000, 10);
        let pick = line(550, 10);
        let (_, overage) = try_fit(
            std::slice::from_ref(&own),
            &[],
            &pick,
            Some(5_000),
            TaxParams::fallback(),
            crate::models::FeeSettings::fallback(),
        )
        .unwrap_err();
        assert_eq!(overage, 550);
        let totals = try_fit(
            &[own],
            &[],
            &pick,
            Some(5_550),
            TaxParams::fallback(),
            crate::models::FeeSettings::fallback(),
        )
        .unwrap();
        assert_eq!(totals.shipping_yen, 0);
        assert_eq!(totals.total_yen, 5_550);
    }

    #[test]
    fn must_have_relaxation_reports_the_unmet_needs() {
        // Nobody tag-matches: everyone stays eligible at the
        // relaxed level and the need is reported unmet.
        let mut b = brief();
        b.must_have = vec!["防水".to_string()];
        let scored = score_candidates(&[candidate(1, 1_000), candidate(2, 2_000)], &b);
        let (eligible, unmet) = eligible_pool(&scored, &b);
        assert_eq!(eligible.len(), 2);
        assert_eq!(unmet, vec!["防水"]);

        // One tag match: only it is eligible, nothing unmet.
        let mut tagged = candidate(3, 3_000);
        tagged.tags = vec![CandidateTag {
            slug: "waterproof".to_string(),
            label: "防水".to_string(),
        }];
        let scored = score_candidates(&[candidate(1, 1_000), tagged], &b);
        let (eligible, unmet) = eligible_pool(&scored, &b);
        assert_eq!(eligible, vec![1]);
        assert!(unmet.is_empty());
    }
}
