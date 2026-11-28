//! Product list and form views.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::AdminProductListRow;
use crate::pricing;
use crate::views::admin::Page;

/// One selectable category in the list filter.
pub struct CategoryOption {
    /// Category value.
    pub value: String,
    /// Whether it is the active filter.
    pub selected: bool,
}

/// One selectable tag in the list filter.
pub struct TagOption {
    /// Tag ID.
    pub id: i64,
    /// Japanese label.
    pub label: String,
    /// Whether it is the active filter.
    pub selected: bool,
}

/// One sort option of the list's sort select.
pub struct SortOption {
    /// Sort key value.
    pub value: &'static str,
    /// Japanese label.
    pub label: &'static str,
    /// Whether it is the active sort.
    pub selected: bool,
}

/// One axis block of the form's variation editor.
pub struct FormAxis {
    /// Field-name index (`axes[<index>]...`).
    pub index: usize,
    /// Stored axis ID; empty for a new axis.
    pub id: String,
    /// Header text, as typed.
    pub name: String,
    /// The axis options.
    pub options: Vec<FormOption>,
}

/// One option input of the variation editor.
pub struct FormOption {
    /// Field-name index within its axis.
    pub index: usize,
    /// Stored option ID; empty for a new option.
    pub id: String,
    /// Option text, as typed.
    pub value: String,
}

/// One checkbox row of the form's tag list.
pub struct TagCheck {
    /// Tag ID (the checkbox value).
    pub id: i64,
    /// Japanese label.
    pub label: String,
    /// Whether the box starts checked.
    pub checked: bool,
}

/// One attached image of the form's gallery, in display
/// order: the first entry is the main image.
pub struct FormImage {
    /// Served path: the thumbnail, the preview source and
    /// the submitted `images` value in one.
    pub path: String,
    /// The image's alt text, edited beside the thumbnail.
    pub alt: String,
}

/// One row of the product list, with display text precomputed
/// (templates never format prices or times themselves).
pub struct ProductRow {
    /// The product, in the list's narrow row shape.
    pub product: AdminProductListRow,
    /// Formatted price.
    pub price_text: String,
    /// Formatted last-update time, `YYYY/MM/DD HH:mm` JST.
    pub updated_text: String,
}

/// The admin product list page.
#[derive(Template)]
#[template(path = "admin/pages/products/list.html")]
pub struct ProductListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Active tab key (`all`, `public`, `draft`, `archived`).
    pub tab: String,
    /// Tab counts: all (excl. archived), public, draft, archived.
    pub count_all: i64,
    /// Public tab count.
    pub count_public: i64,
    /// Draft tab count.
    pub count_draft: i64,
    /// Archived tab count.
    pub count_archived: i64,
    /// Tab URLs: same filters, that tab, page 1.
    pub tab_all_url: String,
    /// Public tab URL.
    pub tab_public_url: String,
    /// Draft tab URL.
    pub tab_draft_url: String,
    /// Archived tab URL.
    pub tab_archived_url: String,
    /// CSV export URL carrying the current tab and filters.
    pub csv_export_url: String,
    /// List URL carrying the current tab, filters and page:
    /// the bulk form posts it back as `return_url`, so a bulk
    /// action returns the operator to the exact list they
    /// were working in.
    pub bulk_return_url: String,
    /// The keyword as typed.
    pub q: String,
    /// The rows of this page.
    pub products: Vec<ProductRow>,
    /// Category filter options.
    pub categories: Vec<CategoryOption>,
    /// Tag filter options.
    pub tags: Vec<TagOption>,
    /// Sort options.
    pub sorts: Vec<SortOption>,
    /// Pagination state.
    pub page: Page,
}

/// One option of the product form's category select.
pub struct FormCategoryOption {
    /// Category ID (the submitted value).
    pub id: i64,
    /// Category name.
    pub name: String,
    /// Whether the category is inactive — it appears only as
    /// the product's own current category, marked 無効.
    pub inactive: bool,
    /// Whether it is the current selection.
    pub selected: bool,
}

/// The shared create/edit product form page.
#[derive(Template)]
#[template(path = "admin/pages/products/form.html")]
pub struct ProductFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading （商品を登録 / 商品を編集）.
    pub heading: String,
    /// Storefront detail URL of the product being edited
    /// （ストアで見る）; `None` on the create form, which has no
    /// storefront page yet.
    pub storefront_url: Option<String>,
    /// Why the storefront link is inert — the product is not
    /// public, so its storefront page is a 404 and the link
    /// must say so instead of sending the operator there;
    /// `None` when the product is public and the link is live.
    pub storefront_note: Option<String>,
    /// Form POST target.
    pub action: String,
    /// Staleness token for the edit form: the product's
    /// `updated_at` (epoch microseconds) as loaded, posted
    /// back as the hidden `expected` field; empty on the
    /// create form.
    pub expected: String,
    /// Name field value.
    pub name: String,
    /// Description field value.
    pub description: String,
    /// Category select options: the active categories, plus
    /// the product's own current one when it has been retired.
    pub category_options: Vec<FormCategoryOption>,
    /// Price field value, as typed.
    pub price_yen: String,
    /// Variation editor axis blocks.
    pub variant_axes: Vec<FormAxis>,
    /// JSON of the stored combinations (option values + stock)
    /// for the grid prefill on a fresh edit page.
    pub existing_combos_json: String,
    /// JSON of the combination field sets a failed submit
    /// posted, for the grid refill in posted order; empty on a
    /// fresh page.
    pub grid_combos_json: String,
    /// Every field error by field name, for the sections whose
    /// fields render their error through `error_for`.
    pub errors_map: std::collections::BTreeMap<String, String>,
    /// Tax class select options (name + today's rate).
    pub tax_class_options: Vec<TaxClassOption>,
    /// Price entry mode (`incl` / `excl`).
    pub price_mode: String,
    /// Tax-exclusive price field value, as typed.
    pub price_excl_yen: String,
    /// The current price's tax-exclusive equivalent, for the
    /// entry-mode helper's reference display.
    pub price_excl_display: String,
    /// The configured tax rounding mode's name ("floor",
    /// "round" or "ceil"), carried on the tax class select
    /// for the form's linked 税抜/税込 inputs.
    pub tax_rounding: String,
    /// Cost field value, as typed.
    pub cost_yen: String,
    /// Selected tax class ID, as typed.
    pub tax_class_id: String,
    /// Sale price field value, as typed.
    pub sale_price_yen: String,
    /// Sale window start field value.
    pub sale_starts_at: String,
    /// Sale window end field value.
    pub sale_ends_at: String,
    /// Product code field value.
    pub product_code: String,
    /// Brand field value.
    pub brand: String,
    /// Manufacturer field value.
    pub manufacturer: String,
    /// Country of origin field value.
    pub country_of_origin: String,
    /// Specification field value.
    pub spec_text: String,
    /// Features field value.
    pub features_text: String,
    /// Smartphone description field value.
    pub description_sp: String,
    /// Search keywords field value.
    pub search_keywords: String,
    /// 新着 checkbox state.
    pub is_new: bool,
    /// 予約 checkbox state.
    pub is_preorder: bool,
    /// おすすめ checkbox state.
    pub is_staff_pick: bool,
    /// Discount percent field value, as typed.
    pub discount_percent: String,
    /// Display order field value, as typed.
    pub display_order: String,
    /// Slug field value.
    pub slug: String,
    /// SEO title field value.
    pub seo_title: String,
    /// Meta description field value.
    pub meta_description: String,
    /// Stock tracking checkbox state.
    pub track_stock: bool,
    /// Backorder checkbox state.
    pub allow_backorder: bool,
    /// Low-stock threshold field value, as typed.
    pub low_stock_threshold: String,
    /// Points earn-rate override field value, as typed.
    pub points_earn_percent: String,
    /// Weight field value, as typed.
    pub weight_grams: String,
    /// Parcel length field value, as typed.
    pub size_length_mm: String,
    /// Parcel width field value, as typed.
    pub size_width_mm: String,
    /// Parcel height field value, as typed.
    pub size_height_mm: String,
    /// Chilled delivery checkbox state.
    pub cool_delivery: bool,
    /// Mail delivery checkbox state.
    pub mail_delivery: bool,
    /// Free shipping checkbox state.
    pub free_shipping: bool,
    /// Undeliverable regions field value.
    pub undeliverable_regions: String,
    /// Lead time field value, as typed.
    pub lead_time_days: String,
    /// Publication window start field value.
    pub publish_starts_at: String,
    /// Publication window end field value.
    pub publish_ends_at: String,
    /// Stock history page URL (edit form only); empty on create.
    pub stock_history_url: String,
    /// Back-in-stock subscription summary (edit form only,
    /// when any exist); `None` renders no line.
    pub restock_summary: Option<String>,
    /// Status select value (`public` or `draft`).
    pub status: String,
    /// Event flag checkbox state.
    pub is_event: bool,
    /// Attached images in display order (first is main).
    pub images: Vec<FormImage>,
    /// Tag checkboxes.
    pub tags: Vec<TagCheck>,
    /// Whether the form is locked (an archived product's edit
    /// page: fields render disabled until unarchived).
    pub readonly: bool,
    /// The product's current status label, for the archived
    /// notice; empty when not archived.
    pub status_note: String,
    /// The unarchive form's target, when archived.
    pub unarchive_action: String,
    /// The archive form's target, when the product can be
    /// archived (editing a public/draft product); empty on the
    /// create form and for archived products.
    pub archive_action: String,
    /// Name field error.
    pub name_error: Option<String>,
    /// Description field error.
    pub description_error: Option<String>,
    /// Category field error.
    pub category_error: Option<String>,
    /// Price field error.
    pub price_error: Option<String>,
    /// Variation section error.
    pub variants_error: Option<String>,
    /// Image field error.
    pub image_error: Option<String>,
}

impl ProductFormView {
    /// Whether the status select should show 公開 selected.
    pub fn status_is_public(&self) -> bool {
        self.status == "public"
    }

    /// One field's error message, for the sections that render
    /// errors through the map.
    pub fn error_for(&self, field: &str) -> Option<String> {
        self.errors_map.get(field).cloned()
    }

    /// Whether 税抜 entry mode is selected.
    pub fn price_mode_is_excl(&self) -> bool {
        self.price_mode == "excl"
    }
}

/// One option of the form's tax class select.
pub struct TaxClassOption {
    /// Tax class ID (the submitted value).
    pub id: i64,
    /// Display label: class name with today's rate.
    pub label: String,
    /// Whether it is the current selection.
    pub selected: bool,
    /// The class's rate in whole percent, carried on the
    /// option element for the form's linked 税抜/税込 inputs.
    pub rate_percent: i32,
}

/// Formats a yen amount for the admin lists.
pub fn yen(amount: i64) -> String {
    pricing::display_price(amount)
}

/// The per-product stock history page: the stock ledger's rows
/// for the product's combinations, newest first.
#[derive(Template)]
#[template(path = "admin/pages/products/stock_history.html")]
pub struct StockHistoryView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The product's ID (the back link and title context).
    pub product_id: i64,
    /// The product's name.
    pub product_name: String,
    /// The ledger rows.
    pub rows: Vec<crate::repositories::stock_ledger_repository::StockAdjustment>,
}

/// The CSV import upload page for 商品.
#[derive(Template)]
#[template(path = "admin/pages/products/csv_import.html")]
pub struct ProductCsvImportView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
}

/// The CSV import preview page for 商品: one batch's
/// validation report with its apply/discard actions.
#[derive(Template)]
#[template(path = "admin/pages/products/csv_preview.html")]
pub struct ProductCsvPreviewView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The batch's ID (the apply/discard URLs carry it).
    pub batch_id: i64,
    /// The uploaded file's name.
    pub file_name: String,
    /// Batch lifecycle: `preview`, `applied` or `discarded`.
    pub status: String,
    /// Records the apply step would create.
    pub new_count: usize,
    /// Records the apply step would update.
    pub update_count: usize,
    /// How many problems the validation found.
    pub error_count: usize,
    /// The problems, in file order.
    pub errors: Vec<crate::util::csv_transfer::LineError>,
}
