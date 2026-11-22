//! Storefront coupon service: member coupon reads and the
//! evaluation engine.
//!
//! The engine (Coupon Shopify-Core spec) prices a cart's
//! coupons by combination class — product, order, shipping —
//! with at most one coupon per class: customer-entered codes
//! win their class over automatic discounts, which fill the
//! classes no code claimed. The checkout view and order
//! placement share this one evaluation, so a total can never
//! be priced two ways.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{AppliedCoupon, CartLine, Coupon, MemberCoupon};
use crate::repositories::coupon_repository;
use crate::services::cart_service;

/// Returns every coupon copy the member holds — held and used —
/// with definitions and use state.
pub async fn member_coupons(pool: &PgPool, user_id: i64) -> Result<Vec<MemberCoupon>, AppError> {
    coupon_repository::list_for_member(pool, user_id).await
}

/// What the engine decided for a cart: the applied coupons
/// with their amounts, split by the class totals the money
/// paths consume.
#[derive(Clone, Debug, Default)]
pub struct CouponOutcome {
    /// The applied coupons, product class first, then order,
    /// then shipping.
    pub applied: Vec<AppliedCoupon>,
    /// The product class's discount in yen.
    pub product_discount_yen: i64,
    /// The order class's discount in yen.
    pub order_discount_yen: i64,
    /// The shipping class's discount in yen (the waived fee).
    pub shipping_discount_yen: i64,
}

impl CouponOutcome {
    /// Returns the combined product + order discount: the
    /// single discount the totals computation and the tax
    /// allocation consume.
    pub fn discount_yen(&self) -> i64 {
        self.product_discount_yen + self.order_discount_yen
    }

    /// Whether the given code is among the applied coupons.
    pub fn contains(&self, code: &str) -> bool {
        self.applied.iter().any(|a| a.code == code)
    }

    /// Returns the code recorded on the order row: the order
    /// class's code when one applied, else the first applied
    /// code (the member surfaces and the confirmation mail
    /// name a single code, as before).
    pub fn primary_code(&self) -> Option<String> {
        self.applied
            .iter()
            .find(|a| a.class == "order")
            .or_else(|| self.applied.first())
            .map(|a| a.code.clone())
    }
}

/// One candidate for the slotting step: a definition with its
/// cart facts resolved (scope basis, usage counts), so the
/// slotting itself is a pure function the tests drive without
/// a database.
struct Candidate {
    /// The coupon definition.
    def: Coupon,
    /// The yen basis the value prices from: the scoped lines
    /// total, or the subtotal for order scope. `None` when a
    /// product/variant scope's target is not in the cart.
    basis_yen: Option<i64>,
    /// Redemptions recorded against the total cap.
    total_uses: i64,
    /// The member's redemptions against the per-customer cap;
    /// `None` disables that cap (guests and automatic
    /// discounts have no per-customer accounting here).
    user_uses: Option<i64>,
    /// Whether a customer entered this code (vs automatic).
    entered: bool,
}

impl Candidate {
    /// Whether every non-class condition passes: the scope
    /// target is in the cart, the minimum subtotal is met,
    /// and neither usage cap is exhausted.
    fn eligible(&self, subtotal_yen: i64) -> bool {
        if self.basis_yen.is_none() || subtotal_yen < self.def.min_subtotal_yen {
            return false;
        }
        if let Some(max) = self.def.max_uses_total
            && self.total_uses >= i64::from(max)
        {
            return false;
        }
        if let (Some(max), Some(used)) = (self.def.max_uses_per_customer, self.user_uses)
            && used >= i64::from(max)
        {
            return false;
        }
        true
    }
}

/// Slots candidates into classes and prices them. Class
/// order is fixed — product, then order (whose basis is the
/// subtotal after the product discount), then shipping (the
/// fee itself) — and inside a class an entered code beats an
/// automatic discount; the first eligible candidate wins the
/// class. A candidate that prices to zero (a free-shipping
/// coupon on an already-free order) does not apply.
fn resolve(candidates: &[Candidate], subtotal_yen: i64, shipping_fee_yen: i64) -> CouponOutcome {
    let mut outcome = CouponOutcome::default();
    for class in ["product", "order", "shipping"] {
        let mut class_candidates: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| c.def.class() == class)
            .collect();
        // Stable sort: entered codes first, entry order kept
        // inside each group.
        class_candidates.sort_by_key(|c| !c.entered);
        for candidate in class_candidates {
            if !candidate.eligible(subtotal_yen) {
                continue;
            }
            let amount = match class {
                "product" => candidate
                    .def
                    .value_for_basis(candidate.basis_yen.unwrap_or(0)),
                "order" => candidate
                    .def
                    .value_for_basis(subtotal_yen - outcome.product_discount_yen),
                _ => shipping_fee_yen,
            };
            if amount <= 0 {
                continue;
            }
            outcome.applied.push(AppliedCoupon {
                code: candidate.def.code.clone(),
                class: class.to_string(),
                discount_yen: amount,
                automatic: !candidate.entered,
                description: candidate.def.description.clone(),
            });
            match class {
                "product" => outcome.product_discount_yen = amount,
                "order" => outcome.order_discount_yen = amount,
                _ => outcome.shipping_discount_yen = amount,
            }
            break;
        }
    }
    outcome
}

/// Resolves a definition's scope basis against the cart
/// lines: order scope prices from the subtotal; product and
/// variant scopes from their lines' totals, absent (`None`)
/// when the target is not in the cart.
fn scope_basis(def: &Coupon, lines: &[CartLine], subtotal_yen: i64) -> Option<i64> {
    match def.scope.as_str() {
        "product" => {
            let sum: i64 = lines
                .iter()
                .filter(|l| Some(l.product_id) == def.product_id)
                .map(|l| l.line_total_yen())
                .sum();
            (sum > 0).then_some(sum)
        }
        "variant" => {
            let sum: i64 = lines
                .iter()
                .filter(|l| Some(l.variant_id) == def.variant_id)
                .map(|l| l.line_total_yen())
                .sum();
            (sum > 0).then_some(sum)
        }
        _ => Some(subtotal_yen),
    }
}

/// Canonicalises an entered code (trimmed, uppercased: the
/// D1 rule the definition writes enforce).
fn canonical(code: &str) -> String {
    code.trim().to_uppercase()
}

/// Evaluates a cart's coupons against the pool: the entered
/// codes (members only — a guest's codes are refused by
/// omission, exactly like an invalid code) plus every active
/// automatic discount. A failed read is logged and the
/// candidate is dropped, never priced.
pub async fn evaluate(
    pool: &PgPool,
    user_id: Option<i64>,
    lines: &[CartLine],
    codes: &[String],
) -> CouponOutcome {
    let purchasable: Vec<CartLine> = lines.iter().filter(|l| l.purchasable()).cloned().collect();
    let subtotal: i64 = purchasable.iter().map(|l| l.line_total_yen()).sum();
    let mut candidates: Vec<Candidate> = Vec::new();
    // The entered codes, canonicalised and de-duplicated in
    // entry order (entering codes is a member-only input).
    let mut entered_codes: Vec<String> = Vec::new();
    if user_id.is_some() {
        for raw in codes {
            let code = canonical(raw);
            if !code.is_empty() && !entered_codes.contains(&code) {
                entered_codes.push(code);
            }
        }
    }
    // Every definition loads in two queries and every
    // redemption count in two grouped queries: the cart
    // page evaluates on every view, and the per-code reads
    // this replaces cost three queries per code.
    let entered_defs = match coupon_repository::find_active_many(pool, &entered_codes).await {
        Ok(defs) => defs,
        Err(e) => {
            tracing::error!("coupon lookup failed: {e}");
            Vec::new()
        }
    };
    let automatic_defs = match coupon_repository::list_automatics(pool).await {
        Ok(defs) => defs,
        Err(e) => {
            tracing::error!("automatic coupon lookup failed: {e}");
            Vec::new()
        }
    };
    let mut count_codes: Vec<String> = entered_defs.iter().map(|d| d.code.clone()).collect();
    for def in &automatic_defs {
        if !count_codes.contains(&def.code) {
            count_codes.push(def.code.clone());
        }
    }
    // A failed count query reads as the cap reached for
    // every code it covered — fail closed, exactly as the
    // per-code count reads degraded.
    let total_counts: HashMap<String, i64> =
        match coupon_repository::redemption_counts(pool, &count_codes).await {
            Ok(rows) => rows.into_iter().collect(),
            Err(_) => count_codes.iter().map(|c| (c.clone(), i64::MAX)).collect(),
        };
    let user_counts: HashMap<String, i64> = match user_id {
        Some(uid) => {
            match coupon_repository::redemption_counts_for_user(pool, uid, &count_codes).await {
                Ok(rows) => rows.into_iter().collect(),
                Err(_) => count_codes.iter().map(|c| (c.clone(), i64::MAX)).collect(),
            }
        }
        None => HashMap::new(),
    };
    let mut entered_by_code: HashMap<String, Coupon> = entered_defs
        .into_iter()
        .map(|d| (d.code.clone(), d))
        .collect();
    for code in &entered_codes {
        let Some(def) = entered_by_code.remove(code) else {
            continue;
        };
        candidates.push(Candidate {
            basis_yen: scope_basis(&def, &purchasable, subtotal),
            total_uses: total_counts.get(code).copied().unwrap_or(0),
            user_uses: Some(user_counts.get(code).copied().unwrap_or(0)),
            def,
            entered: true,
        });
    }
    for def in automatic_defs {
        if user_id.is_some() && candidates.iter().any(|c| c.def.code == def.code) {
            continue;
        }
        candidates.push(Candidate {
            basis_yen: scope_basis(&def, &purchasable, subtotal),
            total_uses: total_counts.get(&def.code).copied().unwrap_or(0),
            user_uses: user_id.map(|_| user_counts.get(&def.code).copied().unwrap_or(0)),
            def,
            entered: false,
        });
    }

    resolve(
        &candidates,
        subtotal,
        cart_service::shipping_fee_for(
            &purchasable,
            crate::services::shop_settings_service::load(pool)
                .await
                .fee_settings(),
        ),
    )
}

/// Evaluates a cart's coupons inside the placement
/// transaction, with every candidate definition locked: the
/// caps the view promised are re-checked against the ledger
/// under the lock, so racing placements cannot overshoot
/// them.
///
/// # Errors
///
/// Returns the database error of any read; placement maps
/// it to its database failure.
pub async fn evaluate_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Option<i64>,
    lines: &[CartLine],
    codes: &[String],
    fees: crate::models::FeeSettings,
) -> Result<CouponOutcome, sqlx::Error> {
    let purchasable: Vec<CartLine> = lines.iter().filter(|l| l.purchasable()).cloned().collect();
    let subtotal: i64 = purchasable.iter().map(|l| l.line_total_yen()).sum();
    let mut candidates: Vec<Candidate> = Vec::new();
    if let Some(uid) = user_id {
        let mut seen: Vec<String> = Vec::new();
        for raw in codes {
            let code = canonical(raw);
            if code.is_empty() || seen.contains(&code) {
                continue;
            }
            seen.push(code.clone());
            if let Some(def) = coupon_repository::find_active_for_update(tx, &code).await? {
                let total_uses = coupon_repository::count_redemptions(&mut **tx, &code).await?;
                let user_uses =
                    coupon_repository::count_redemptions_for_user(&mut **tx, uid, &code).await?;
                candidates.push(Candidate {
                    basis_yen: scope_basis(&def, &purchasable, subtotal),
                    def,
                    total_uses,
                    user_uses: Some(user_uses),
                    entered: true,
                });
            }
        }
    }
    for def in coupon_repository::list_automatics_for_update(tx).await? {
        if user_id.is_some() && candidates.iter().any(|c| c.def.code == def.code) {
            continue;
        }
        let total_uses = coupon_repository::count_redemptions(&mut **tx, &def.code).await?;
        let user_uses = match user_id {
            Some(uid) => Some(
                coupon_repository::count_redemptions_for_user(&mut **tx, uid, &def.code).await?,
            ),
            None => None,
        };
        candidates.push(Candidate {
            basis_yen: scope_basis(&def, &purchasable, subtotal),
            def,
            total_uses,
            user_uses,
            entered: false,
        });
    }
    let outcome = resolve(
        &candidates,
        subtotal,
        cart_service::shipping_fee_for(&purchasable, fees),
    );
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a definition with the given pricing shape.
    fn def(code: &str, discount_type: &str, scope: &str) -> Coupon {
        Coupon {
            code: code.to_string(),
            discount_type: discount_type.to_string(),
            discount_yen: 0,
            percent: None,
            min_subtotal_yen: 0,
            scope: scope.to_string(),
            product_id: None,
            variant_id: None,
            max_uses_total: None,
            max_uses_per_customer: Some(1),
            is_automatic: false,
            description: String::new(),
        }
    }

    /// Builds a candidate that is present and unused.
    fn candidate(def: Coupon, basis: Option<i64>, entered: bool) -> Candidate {
        Candidate {
            def,
            basis_yen: basis,
            total_uses: 0,
            user_uses: Some(0),
            entered,
        }
    }

    #[test]
    fn product_percent_prices_from_the_scoped_basis() {
        let mut d = def("P10", "percent", "product");
        d.percent = Some(10);
        let out = resolve(&[candidate(d, Some(2_000), true)], 5_000, 550);
        assert_eq!(out.product_discount_yen, 200);
        assert_eq!(out.applied.len(), 1);
    }

    #[test]
    fn order_percent_prices_after_the_product_discount() {
        let mut p = def("P500", "fixed", "product");
        p.discount_yen = 500;
        let mut o = def("O10", "percent", "order");
        o.percent = Some(10);
        let out = resolve(
            &[
                candidate(p, Some(3_000), true),
                candidate(o, Some(5_000), true),
            ],
            5_000,
            550,
        );
        // (5000 - 500) * 10% = 450.
        assert_eq!(out.order_discount_yen, 450);
        assert_eq!(out.discount_yen(), 950);
    }

    #[test]
    fn entered_code_beats_automatic_in_the_same_class() {
        let mut entered = def("ENTERED", "fixed", "order");
        entered.discount_yen = 300;
        let mut auto = def("AUTO", "fixed", "order");
        auto.discount_yen = 900;
        auto.is_automatic = true;
        let out = resolve(
            &[
                candidate(auto, Some(5_000), false),
                candidate(entered, Some(5_000), true),
            ],
            5_000,
            550,
        );
        assert_eq!(out.applied.len(), 1);
        assert_eq!(out.applied[0].code, "ENTERED");
        assert_eq!(out.order_discount_yen, 300);
    }

    #[test]
    fn three_classes_combine_on_one_cart() {
        let mut p = def("P", "fixed", "product");
        p.discount_yen = 200;
        let mut o = def("O", "fixed", "order");
        o.discount_yen = 300;
        let ship = def("S", "free_shipping", "order");
        let out = resolve(
            &[
                candidate(p, Some(1_000), true),
                candidate(o, Some(4_000), true),
                candidate(ship, Some(4_000), true),
            ],
            4_000,
            550,
        );
        assert_eq!(out.applied.len(), 3);
        assert_eq!(out.discount_yen(), 500);
        assert_eq!(out.shipping_discount_yen, 550);
    }

    #[test]
    fn free_shipping_does_not_apply_when_shipping_is_already_free() {
        let ship = def("S", "free_shipping", "order");
        let out = resolve(&[candidate(ship, Some(6_000), true)], 6_000, 0);
        assert!(out.applied.is_empty());
        assert_eq!(out.shipping_discount_yen, 0);
    }

    #[test]
    fn caps_and_minimum_block_a_candidate() {
        let mut capped = def("CAPPED", "fixed", "order");
        capped.discount_yen = 500;
        capped.max_uses_per_customer = Some(1);
        let mut c = candidate(capped, Some(5_000), true);
        c.user_uses = Some(1);
        let out = resolve(&[c], 5_000, 550);
        assert!(out.applied.is_empty());

        let mut low = def("LOW", "fixed", "order");
        low.discount_yen = 500;
        low.min_subtotal_yen = 9_999;
        let out = resolve(&[candidate(low, Some(5_000), true)], 5_000, 550);
        assert!(out.applied.is_empty());
    }

    #[test]
    fn absent_scope_target_blocks_a_product_coupon() {
        let mut p = def("P", "fixed", "product");
        p.discount_yen = 500;
        let out = resolve(&[candidate(p, None, true)], 5_000, 550);
        assert!(out.applied.is_empty());
    }
}
