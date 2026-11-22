//! Product variant models: free-form variation axes, their
//! options, and the combination rows that carry stock.
//!
//! A product's variations are named by the shop itself (サイズ,
//! カラー, 容量, ...): each axis has options, and one option per
//! axis forms a combination — a [`ProductVariant`] — which is
//! the unit the cart, stock locks and orders all key on.

/// One variation axis of a product (a header like サイズ), as
/// stored in `product_variant_axes`.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct VariantAxis {
    /// Surrogate ID.
    pub id: i64,
    /// Product the axis belongs to.
    pub product_id: i64,
    /// The header text the shop typed, shown above the pill group.
    pub name: String,
    /// Display order among the product's axes.
    pub sort_order: i32,
}

/// One option of a variation axis (a choice like `L`), as stored
/// in `product_variant_options`.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct VariantOption {
    /// Surrogate ID.
    pub id: i64,
    /// Axis the option belongs to.
    pub axis_id: i64,
    /// The choice text the shop typed.
    pub value: String,
    /// Display order among the axis's options.
    pub sort_order: i32,
}

/// One axis with its options, in display order: the unit the
/// detail-page picker and the admin editor both render.
#[derive(Clone, Debug)]
pub struct AxisWithOptions {
    /// The axis.
    pub axis: VariantAxis,
    /// Its options, in display order.
    pub options: Vec<VariantOption>,
}

/// One combination of a product — one option per axis — as
/// stored in `product_variants`. The default variant of an
/// axis-less product has the empty signature and no option links.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ProductVariant {
    /// Surrogate ID; the cart and order lines key on it.
    pub id: i64,
    /// Product the combination belongs to.
    pub product_id: i64,
    /// The combination's option IDs in axis order, `-`-joined;
    /// empty for the default variant. Unique per product.
    pub option_signature: String,
    /// Units in stock for this combination. Stock lives here and
    /// nowhere else: a product's stock is the sum over its variants.
    pub stock: i32,
    /// This combination's own price (tax included); `None` sells
    /// at the product's effective price.
    pub price_yen: Option<i64>,
    /// This combination's SKU, when registered.
    pub sku: Option<String>,
    /// This combination's JAN code, when registered.
    pub jan_code: Option<String>,
    /// This combination's image: one of the product's gallery
    /// paths, when registered.
    pub image_path: Option<String>,
    /// Whether the combination is offered at all: a disabled
    /// combination shows in the picker as unavailable and can
    /// never be added or ordered.
    pub is_enabled: bool,
    /// Units expected to arrive （入荷予定数）, admin-only bookkeeping.
    pub incoming_qty: i32,
    /// Expected arrival date （入荷予定日）, admin-only bookkeeping.
    pub incoming_date: Option<chrono::NaiveDate>,
}

impl ProductVariant {
    /// Whether this is the product's default variant (the product
    /// has no axes, so this is its only combination).
    pub fn is_default(&self) -> bool {
        self.option_signature.is_empty()
    }
}

/// Builds a combination signature from option IDs in axis order:
/// the `-`-joined IDs, or the empty string when there are none
/// (the default variant). The one signature builder, shared by
/// the admin save, the detail picker and placement, so the three
/// can never derive different keys for the same combination.
///
/// # Examples
///
/// ```
/// use app::models::variant::signature_for;
///
/// assert_eq!(signature_for(&[3, 7]), "3-7");
/// assert_eq!(signature_for(&[]), "");
/// ```
pub fn signature_for(option_ids: &[i64]) -> String {
    option_ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join("-")
}

/// Every combination of the per-axis lists, in grid order: the
/// first axis varies slowest, the last fastest. The admin stock
/// grid, the posted stock order, the combination regeneration
/// and the detail picker all walk this one order.
///
/// # Examples
///
/// ```
/// use app::models::variant::cartesian;
///
/// assert_eq!(cartesian(&[vec![1, 2], vec![3]]), vec![vec![1, 3], vec![2, 3]]);
/// assert_eq!(cartesian::<i64>(&[]), vec![Vec::<i64>::new()]);
/// ```
pub fn cartesian<T: Clone>(per_axis: &[Vec<T>]) -> Vec<Vec<T>> {
    let mut combos: Vec<Vec<T>> = vec![Vec::new()];
    for axis in per_axis {
        let mut next = Vec::new();
        for combo in &combos {
            for item in axis {
                let mut extended = combo.clone();
                extended.push(item.clone());
                next.push(extended);
            }
        }
        combos = next;
    }
    combos
}

/// Builds the display label of one combination from its
/// (axis name, option value) pairs in axis order:
/// `サイズ: L / カラー: ブラック`. No pairs — the default
/// variant — yields the empty string, and labels render only
/// when non-empty, so plain products show no label anywhere.
/// This is the one label builder: the cart derives it live and
/// placement freezes it onto the order line.
pub fn label_for(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(axis, value)| format!("{axis}: {value}"))
        .collect::<Vec<_>>()
        .join(" / ")
}
