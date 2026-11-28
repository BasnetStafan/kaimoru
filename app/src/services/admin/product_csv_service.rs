//! Product CSV import/export for the admin.
//!
//! Export renders one row per variant (the product columns
//! repeat) for the products the list's current filters match.
//! Import is a three-step flow over a stored batch: the
//! upload parses and validates everything into a report, the
//! preview shows it, and the apply step writes each valid
//! product through the SAME validated save path the product
//! form uses (`validate_product` → `create_product` /
//! `update_product`) — so the publish gate, the variation
//! matrix rules (including the ordered-combination removal
//! protection) and the stock ledger all behave exactly as
//! they do for a form submit. The unit of application is the
//! product: a group of rows with any error applies not at
//! all, and its errors are reported per file line.
//!
//! Category and tax class are referenced by NAME and tags by
//! slug; an unknown name is an error, never an auto-create —
//! a typo in a spreadsheet must not mint vocabulary. Columns
//! the CSV map does not carry (the badge flags, the discount
//! percent) are carried forward from the stored product on
//! update, so a round-trip never silently clears them.

use std::collections::{HashMap, HashSet};

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::messages::csv_row_limit_message;
use crate::models::admin_rows::AdminProduct;
use crate::repositories::{csv_import_batch_repository, tax_repository};
use crate::services::admin::product_service::{
    self, AxisFormInput, ComboFormInput, ExistingMatrix, OptionFormInput, ProductFormInput,
    ValidProduct,
};
use crate::util::csv_transfer::{
    BatchReport, CsvRow, LineError, decode_bytes, parse_rows, write_csv,
};

/// The fixed product column set, in file order. Export and
/// import share this one definition.
pub const PRODUCT_HEADERS: [&str; 59] = [
    "商品ID",
    "商品コード",
    "商品名",
    "税込価格",
    "セール価格",
    "セール開始",
    "セール終了",
    "原価",
    "消費税区分",
    "ステータス",
    "カテゴリー",
    "タグ",
    "商品説明",
    "商品説明（SP）",
    "仕様",
    "特長",
    "ブランド",
    "メーカー",
    "原産国",
    "検索キーワード",
    "スラッグ",
    "SEOタイトル",
    "メタ説明",
    "おすすめ",
    "表示順",
    "公開開始",
    "公開終了",
    "在庫管理",
    "在庫切れ時販売",
    "在庫僅少 しきい値",
    "重量g",
    "サイズ縦mm",
    "サイズ横mm",
    "サイズ高さmm",
    "クール便",
    "メール便",
    "送料無料対象",
    "配送不可地域",
    "発送リードタイム",
    "画像パス一覧",
    "バリエーション1名",
    "バリエーション1値",
    "バリエーション2名",
    "バリエーション2値",
    "バリエーション3名",
    "バリエーション3値",
    "バリエーション4名",
    "バリエーション4値",
    "バリエーション5名",
    "バリエーション5値",
    "バリエーション価格",
    "SKU",
    "JANコード",
    "在庫数",
    "バリエーション有効",
    "入荷予定数",
    "入荷予定日",
    "バリエーション画像パス",
    "ポイント付与率",
];

// Column indices into PRODUCT_HEADERS.
const C_ID: usize = 0;
const C_CODE: usize = 1;
const C_NAME: usize = 2;
const C_PRICE: usize = 3;
const C_SALE_PRICE: usize = 4;
const C_SALE_START: usize = 5;
const C_SALE_END: usize = 6;
const C_COST: usize = 7;
const C_TAX: usize = 8;
const C_STATUS: usize = 9;
const C_CATEGORY: usize = 10;
const C_TAGS: usize = 11;
const C_DESC: usize = 12;
const C_DESC_SP: usize = 13;
const C_SPEC: usize = 14;
const C_FEATURES: usize = 15;
const C_BRAND: usize = 16;
const C_MAKER: usize = 17;
const C_COUNTRY: usize = 18;
const C_KEYWORDS: usize = 19;
const C_SLUG: usize = 20;
const C_SEO_TITLE: usize = 21;
const C_META: usize = 22;
const C_FEATURED: usize = 23;
const C_ORDER: usize = 24;
const C_PUB_START: usize = 25;
const C_PUB_END: usize = 26;
const C_TRACK: usize = 27;
const C_BACKORDER: usize = 28;
const C_LOW_STOCK: usize = 29;
const C_WEIGHT: usize = 30;
const C_SIZE_L: usize = 31;
const C_SIZE_W: usize = 32;
const C_SIZE_H: usize = 33;
const C_COOL: usize = 34;
const C_MAIL: usize = 35;
const C_FREE_SHIP: usize = 36;
const C_UNDELIVERABLE: usize = 37;
const C_LEAD: usize = 38;
const C_IMAGES: usize = 39;
const C_AXIS1_NAME: usize = 40;
const C_VAR_PRICE: usize = 50;
const C_SKU: usize = 51;
const C_JAN: usize = 52;
const C_STOCK: usize = 53;
const C_VAR_ENABLED: usize = 54;
const C_INCOMING_QTY: usize = 55;
const C_INCOMING_DATE: usize = 56;
const C_VAR_IMAGE: usize = 57;
const C_POINTS: usize = 58;

/// Formats a stored UTC instant the way the CSV carries
/// datetimes: JST wall clock, `YYYY/MM/DD HH:MM`.
fn fmt_dt(at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    at.map(|dt| {
        dt.with_timezone(&crate::models::jst::jst())
            .format("%Y/%m/%d %H:%M")
            .to_string()
    })
    .unwrap_or_default()
}

/// Renders the export file for the products matching the
/// list query (the controller builds it from the list's
/// current tab and filters, unpaginated).
pub async fn export_csv(
    pool: &PgPool,
    query: &product_service::AdminProductQuery<'_>,
) -> Result<Vec<u8>, AppError> {
    let (products, _) = product_service::list_products(pool, query).await?;
    let classes = tax_repository::class_rates_on(pool, crate::models::jst::jst_today()).await?;
    let tax_names: HashMap<i64, String> = classes
        .iter()
        .map(|c| (c.class.id, c.class.name.clone()))
        .collect();
    let categories = product_service::list_category_records(pool).await?;
    let category_names: HashMap<i64, String> =
        categories.iter().map(|c| (c.id, c.name.clone())).collect();
    let tags = product_service::list_tags(pool).await?;
    let tag_slugs: HashMap<i64, String> = tags.iter().map(|t| (t.id, t.slug.clone())).collect();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for product in &products {
        let combos = product_service::combos_full(pool, product.id).await?;
        let matrix = product_service::existing_matrix(pool, product.id).await?;
        let images = product_service::product_images_detailed(pool, product.id).await?;
        let image_list = images
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>()
            .join("|");
        let tag_list = product_service::product_tag_ids(pool, product.id)
            .await?
            .iter()
            .filter_map(|id| tag_slugs.get(id))
            .cloned()
            .collect::<Vec<_>>()
            .join("|");
        let category_name = product
            .category_id
            .and_then(|id| category_names.get(&id).cloned())
            .unwrap_or_else(|| product.category.clone());
        let head = product_head_cells(
            product,
            tax_names
                .get(&product.tax_class_id)
                .cloned()
                .unwrap_or_default(),
            category_name,
            tag_list,
            image_list,
        );
        if combos.is_empty() {
            // A product always has at least the default
            // variant; an empty read is a data problem, but
            // the export must not invent a stock figure —
            // emit the product columns with empty variant
            // cells so the row is visible and fixable.
            rows.push(head);
            continue;
        }
        for combo in &combos {
            let mut row = head.clone();
            for (i, axis) in matrix.axes.iter().enumerate().take(5) {
                row[C_AXIS1_NAME + i * 2] = axis.axis.name.clone();
                row[C_AXIS1_NAME + i * 2 + 1] = combo.values.get(i).cloned().unwrap_or_default();
            }
            row[C_VAR_PRICE] = combo.price_yen.map(|v| v.to_string()).unwrap_or_default();
            row[C_SKU] = combo.sku.clone();
            row[C_JAN] = combo.jan_code.clone();
            row[C_STOCK] = combo.stock.to_string();
            row[C_VAR_ENABLED] = if combo.is_enabled { "1" } else { "0" }.to_string();
            row[C_INCOMING_QTY] = if combo.incoming_qty > 0 {
                combo.incoming_qty.to_string()
            } else {
                String::new()
            };
            row[C_INCOMING_DATE] = combo.incoming_date.replace('-', "/");
            row[C_VAR_IMAGE] = combo.image_path.clone();
            rows.push(row);
        }
    }
    Ok(write_csv(&PRODUCT_HEADERS, &rows))
}

/// The product-level cells (everything except the variant
/// block), as one row template the variant rows clone.
fn product_head_cells(
    product: &AdminProduct,
    tax_name: String,
    category_name: String,
    tag_list: String,
    image_list: String,
) -> Vec<String> {
    let mut row = vec![String::new(); PRODUCT_HEADERS.len()];
    let b = |v: bool| if v { "1" } else { "0" }.to_string();
    let num = |v: Option<i32>| v.map(|n| n.to_string()).unwrap_or_default();
    row[C_ID] = product.id.to_string();
    row[C_CODE] = product.product_code.clone().unwrap_or_default();
    row[C_NAME] = product.name.clone();
    row[C_PRICE] = product.price_yen.to_string();
    row[C_SALE_PRICE] = product
        .sale_price_yen
        .map(|v| v.to_string())
        .unwrap_or_default();
    row[C_SALE_START] = fmt_dt(product.sale_starts_at);
    row[C_SALE_END] = fmt_dt(product.sale_ends_at);
    row[C_COST] = product.cost_yen.map(|v| v.to_string()).unwrap_or_default();
    row[C_TAX] = tax_name;
    row[C_STATUS] = product.status.clone();
    row[C_CATEGORY] = category_name;
    row[C_TAGS] = tag_list;
    row[C_DESC] = product.description.clone();
    row[C_DESC_SP] = product.description_sp.clone();
    row[C_SPEC] = product.spec_text.clone();
    row[C_FEATURES] = product.features_text.clone();
    row[C_BRAND] = product.brand.clone();
    row[C_MAKER] = product.manufacturer.clone();
    row[C_COUNTRY] = product.country_of_origin.clone();
    row[C_KEYWORDS] = product.search_keywords.clone();
    row[C_SLUG] = product.slug.clone().unwrap_or_default();
    row[C_SEO_TITLE] = product.seo_title.clone();
    row[C_META] = product.meta_description.clone();
    row[C_FEATURED] = b(product.is_staff_pick);
    row[C_ORDER] = product.display_order.to_string();
    row[C_PUB_START] = fmt_dt(product.publish_starts_at);
    row[C_PUB_END] = fmt_dt(product.publish_ends_at);
    row[C_TRACK] = b(product.track_stock);
    row[C_BACKORDER] = b(product.allow_backorder);
    row[C_LOW_STOCK] = num(product.low_stock_threshold);
    row[C_POINTS] = num(product.points_earn_percent);
    row[C_WEIGHT] = num(product.weight_grams);
    row[C_SIZE_L] = num(product.size_length_mm);
    row[C_SIZE_W] = num(product.size_width_mm);
    row[C_SIZE_H] = num(product.size_height_mm);
    row[C_COOL] = b(product.cool_delivery);
    row[C_MAIL] = b(product.mail_delivery);
    row[C_FREE_SHIP] = b(product.free_shipping);
    row[C_UNDELIVERABLE] = product.undeliverable_regions.clone();
    row[C_LEAD] = num(product.lead_time_days);
    row[C_IMAGES] = image_list;
    row
}

/// One product's validated write, ready for the apply step.
pub struct ValidGroup {
    /// Target product ID; `None` creates a new product.
    pub product_id: Option<i64>,
    /// The validated form, exactly as the form path writes it.
    pub valid: ValidProduct,
    /// The CSV's status token (`public`/`draft`/`archived`):
    /// the form path cannot produce `archived`, so the apply
    /// step archives after the write when this asks for it.
    pub desired_status: String,
}

/// A group of rows forming one product, before validation.
struct Group {
    /// File line of the group's first row.
    first_line: usize,
    /// The group's rows, in file order.
    rows: Vec<CsvRow>,
    /// Parsed 商品ID; `None` for a new product.
    product_id: Option<i64>,
    /// Problems found while grouping (malformed or scattered
    /// IDs), which already disqualify the group.
    errors: Vec<LineError>,
}

/// Splits parsed rows into per-product groups. Rows carrying
/// a 商品ID group by that ID (and must be consecutive); rows
/// without one group as a run whose product-level cells match
/// the run's first row — that is how a new multi-variant
/// product is expressed.
fn group_rows(rows: Vec<CsvRow>) -> Vec<Group> {
    /// The product-level fingerprint of a blank-ID row: every
    /// column except the variant block (axes + combo cells).
    fn fingerprint(row: &CsvRow) -> Vec<String> {
        (0..PRODUCT_HEADERS.len())
            .filter(|i| !(C_AXIS1_NAME..=C_VAR_IMAGE).contains(i))
            .map(|i| row.get(i).trim().to_string())
            .collect()
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut closed_ids: HashSet<i64> = HashSet::new();
    for row in rows {
        let id_cell = row.get(C_ID).trim();
        let parsed_id: Option<i64> = if id_cell.is_empty() {
            None
        } else {
            id_cell.parse::<i64>().ok().filter(|n| *n > 0)
        };
        let malformed_id = !id_cell.is_empty() && parsed_id.is_none();

        let continues = match groups.last() {
            Some(last) if !malformed_id => match (last.product_id, parsed_id) {
                (Some(a), Some(b)) => a == b,
                (None, None) => fingerprint(&last.rows[0]) == fingerprint(&row),
                _ => false,
            },
            _ => false,
        };
        if continues {
            groups.last_mut().expect("last group").rows.push(row);
            continue;
        }
        if let Some(last) = groups.last()
            && let Some(id) = last.product_id
        {
            closed_ids.insert(id);
        }
        let mut errors = Vec::new();
        if malformed_id {
            errors.push(LineError::new(
                row.line,
                format!("商品ID「{id_cell}」が正しくありません"),
            ));
        }
        if let Some(id) = parsed_id
            && closed_ids.contains(&id)
        {
            errors.push(LineError::new(
                row.line,
                format!("商品ID {id} の行が離れた場所にもあります（同じ商品の行は続けてください）"),
            ));
        }
        let line = row.line;
        groups.push(Group {
            first_line: line,
            product_id: if malformed_id { None } else { parsed_id },
            rows: vec![row],
            errors,
        });
    }
    groups
}

/// Converts one CSV datetime cell (`YYYY/MM/DD HH:MM`) into
/// the form's datetime-local shape; a malformed value is a
/// line error naming the column.
fn conv_dt(cell: &str, line: usize, column: &str, errors: &mut Vec<LineError>) -> String {
    let cell = cell.trim();
    if cell.is_empty() {
        return String::new();
    }
    match chrono::NaiveDateTime::parse_from_str(cell, "%Y/%m/%d %H:%M") {
        Ok(dt) => dt.format("%Y-%m-%dT%H:%M").to_string(),
        Err(_) => {
            errors.push(LineError::new(
                line,
                format!("{column}の形式が正しくありません（YYYY/MM/DD HH:MM）"),
            ));
            String::new()
        }
    }
}

/// Converts one CSV date cell (`YYYY/MM/DD`) into the form's
/// `YYYY-MM-DD` shape.
fn conv_date(cell: &str, line: usize, column: &str, errors: &mut Vec<LineError>) -> String {
    let cell = cell.trim();
    if cell.is_empty() {
        return String::new();
    }
    match chrono::NaiveDate::parse_from_str(cell, "%Y/%m/%d") {
        Ok(d) => d.format("%Y-%m-%d").to_string(),
        Err(_) => {
            errors.push(LineError::new(
                line,
                format!("{column}の形式が正しくありません（YYYY/MM/DD）"),
            ));
            String::new()
        }
    }
}

/// Reads one 1/0 boolean cell; anything else is a line error.
/// An empty cell reads as `empty` (the variant enable flag
/// defaults on, matching the form's new-combination state;
/// product flags default off).
fn conv_bool(
    cell: &str,
    line: usize,
    column: &str,
    empty: bool,
    errors: &mut Vec<LineError>,
) -> bool {
    match cell.trim() {
        "1" => true,
        "0" => false,
        "" => empty,
        _ => {
            errors.push(LineError::new(
                line,
                format!("{column}は 1 または 0 で指定してください"),
            ));
            false
        }
    }
}

/// Validates one upload end to end: returns the applicable
/// product writes plus the report the preview page shows.
/// Nothing is written here — the apply step re-runs this and
/// writes only the groups that still validate.
pub async fn validate_upload(
    pool: &PgPool,
    text: &str,
) -> Result<(Vec<ValidGroup>, BatchReport), AppError> {
    let mut report = BatchReport::default();
    let rows = match parse_rows(text, &PRODUCT_HEADERS) {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(e);
            return Ok((Vec::new(), report));
        }
    };
    if rows.len() > CONFIG.csv_max_rows.max(0) as usize {
        report.errors.push(LineError::new(
            1,
            csv_row_limit_message(CONFIG.csv_max_rows),
        ));
        return Ok((Vec::new(), report));
    }

    let categories = product_service::list_category_records(pool).await?;
    let category_ids: HashMap<String, i64> =
        categories.iter().map(|c| (c.name.clone(), c.id)).collect();
    let tax_classes = tax_repository::class_rates_on(pool, crate::models::jst::jst_today()).await?;
    let tax_ids: HashMap<String, i64> = tax_classes
        .iter()
        .map(|c| (c.class.name.clone(), c.class.id))
        .collect();
    let tags = product_service::list_tags(pool).await?;
    let tag_ids_by_slug: HashMap<String, i64> =
        tags.iter().map(|t| (t.slug.clone(), t.id)).collect();
    let valid_tag_ids: Vec<i64> = tags.iter().map(|t| t.id).collect();

    // File-level uniqueness: two groups in one file may not
    // claim the same product code, slug or SKU — the per-group
    // validator only sees the database, so without this the
    // second claim would fail mid-apply instead of previewing
    // as an error here.
    let mut claimed_codes: HashMap<String, usize> = HashMap::new();
    let mut claimed_slugs: HashMap<String, usize> = HashMap::new();
    let mut claimed_skus: HashMap<String, usize> = HashMap::new();

    let mut valid_groups: Vec<ValidGroup> = Vec::new();
    for group in group_rows(rows) {
        let mut errors = group.errors.clone();
        if let Some(valid) = build_group(
            pool,
            &group,
            &mut errors,
            &categories,
            &category_ids,
            &tax_classes,
            &tax_ids,
            &tag_ids_by_slug,
            &valid_tag_ids,
            &mut claimed_codes,
            &mut claimed_slugs,
            &mut claimed_skus,
        )
        .await?
        {
            if valid.product_id.is_some() {
                report.update_count += 1;
            } else {
                report.new_count += 1;
            }
            valid_groups.push(valid);
        }
        report.errors.extend(errors);
    }
    report.errors.sort_by_key(|e| e.line);
    Ok((valid_groups, report))
}

/// Builds and validates one product group. Returns the write
/// when the group is clean; every problem found is pushed
/// into `errors` (and the group then applies not at all).
#[allow(clippy::too_many_arguments)]
async fn build_group(
    pool: &PgPool,
    group: &Group,
    errors: &mut Vec<LineError>,
    categories: &[crate::models::admin_rows::ProductCategory],
    category_ids: &HashMap<String, i64>,
    tax_classes: &[crate::models::tax::ClassRate],
    tax_ids: &HashMap<String, i64>,
    tag_ids_by_slug: &HashMap<String, i64>,
    valid_tag_ids: &[i64],
    claimed_codes: &mut HashMap<String, usize>,
    claimed_slugs: &mut HashMap<String, usize>,
    claimed_skus: &mut HashMap<String, usize>,
) -> Result<Option<ValidGroup>, AppError> {
    let first = &group.rows[0];
    let line = group.first_line;

    // The stored product (update target) and its matrix, for
    // the carried-forward fields and the option ID mapping.
    let existing_product = match group.product_id {
        Some(id) => match product_service::find_product(pool, id).await? {
            Some(p) => Some(p),
            None => {
                errors.push(LineError::new(
                    line,
                    format!("商品ID {id} の商品が存在しません"),
                ));
                None
            }
        },
        None => None,
    };
    if group.product_id.is_some() && existing_product.is_none() {
        return Ok(None);
    }
    let existing_matrix: Option<ExistingMatrix> = match &existing_product {
        Some(p) => Some(product_service::existing_matrix(pool, p.id).await?),
        None => None,
    };

    // Name resolutions: unknown names are errors, never
    // auto-creates.
    let category_name = first.get(C_CATEGORY).trim();
    let category_id = if category_name.is_empty() {
        errors.push(LineError::new(line, "カテゴリーを指定してください"));
        None
    } else {
        match category_ids.get(category_name) {
            Some(id) => Some(*id),
            None => {
                errors.push(LineError::new(
                    line,
                    format!("カテゴリー「{category_name}」が存在しません"),
                ));
                None
            }
        }
    };
    let tax_name = first.get(C_TAX).trim();
    let tax_class_id = match tax_ids.get(tax_name) {
        Some(id) => Some(*id),
        None => {
            errors.push(LineError::new(
                line,
                format!("消費税区分「{tax_name}」が存在しません"),
            ));
            None
        }
    };
    let mut tag_ids: Vec<i64> = Vec::new();
    for slug in first
        .get(C_TAGS)
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match tag_ids_by_slug.get(slug) {
            Some(id) => tag_ids.push(*id),
            None => errors.push(LineError::new(
                line,
                format!("タグ「{slug}」が存在しません"),
            )),
        }
    }

    let status_token = first.get(C_STATUS).trim();
    if !matches!(status_token, "public" | "draft" | "archived") {
        errors.push(LineError::new(
            line,
            format!("ステータス「{status_token}」が正しくありません（public / draft / archived）"),
        ));
    }

    // Images: paths, with the stored alt texts carried over
    // for paths the product already carries.
    let images: Vec<String> = first
        .get(C_IMAGES)
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let attached: Vec<String> = match &existing_product {
        Some(p) => product_service::product_images(pool, p.id).await?,
        None => Vec::new(),
    };
    let existing_alts: HashMap<String, String> = match &existing_product {
        Some(p) => product_service::product_images_detailed(pool, p.id)
            .await?
            .into_iter()
            .collect(),
        None => HashMap::new(),
    };
    let image_alts: Vec<String> = images
        .iter()
        .map(|path| existing_alts.get(path).cloned().unwrap_or_default())
        .collect();

    // File-level uniqueness claims (see validate_upload).
    let code = first.get(C_CODE).trim();
    if !code.is_empty() {
        if let Some(first_line) = claimed_codes.get(code) {
            errors.push(LineError::new(
                line,
                format!("商品コード「{code}」が{first_line}行目と重複しています"),
            ));
        } else {
            claimed_codes.insert(code.to_string(), line);
        }
    }
    let slug = first.get(C_SLUG).trim();
    if !slug.is_empty() {
        if let Some(first_line) = claimed_slugs.get(slug) {
            errors.push(LineError::new(
                line,
                format!("スラッグ「{slug}」が{first_line}行目と重複しています"),
            ));
        } else {
            claimed_slugs.insert(slug.to_string(), line);
        }
    }

    // The variation axes: names from the first row, options
    // collected across the group's rows in first-seen order.
    let axis_names: Vec<String> = (0..5)
        .map(|i| first.get(C_AXIS1_NAME + i * 2).trim().to_string())
        .collect();
    let mut axis_options: Vec<Vec<String>> = vec![Vec::new(); 5];
    for row in &group.rows {
        for i in 0..5 {
            let name = row.get(C_AXIS1_NAME + i * 2).trim();
            let value = row.get(C_AXIS1_NAME + i * 2 + 1).trim();
            if name.is_empty() && value.is_empty() {
                continue;
            }
            if name != axis_names[i] {
                errors.push(LineError::new(
                    row.line,
                    "バリエーション名が先頭行と一致しません",
                ));
                continue;
            }
            if value.is_empty() {
                errors.push(LineError::new(
                    row.line,
                    format!("バリエーション{}の値が空です", i + 1),
                ));
                continue;
            }
            if !axis_options[i].contains(&value.to_string()) {
                axis_options[i].push(value.to_string());
            }
        }
    }
    let axis_count = axis_names.iter().filter(|n| !n.is_empty()).count();
    // Axis names must be a prefix: a gap (name 3 set, name 2
    // empty) cannot be expressed by the form's axis list.
    if axis_names
        .iter()
        .enumerate()
        .any(|(i, n)| n.is_empty() && axis_names[i + 1..].iter().any(|m| !m.is_empty()))
    {
        errors.push(LineError::new(
            line,
            "バリエーション名は1から順に指定してください",
        ));
    }

    // Combination rows, keyed by their option-value tuple so
    // the file's row order does not matter; the form's grid
    // order (cartesian, first axis slowest) is rebuilt here.
    let mut combo_rows: HashMap<Vec<String>, &CsvRow> = HashMap::new();
    for (index, row) in group.rows.iter().enumerate() {
        let key: Vec<String> = (0..axis_count)
            .map(|i| row.get(C_AXIS1_NAME + i * 2 + 1).trim().to_string())
            .collect();
        match combo_rows.entry(key) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(row);
            }
            std::collections::hash_map::Entry::Occupied(_) if axis_count > 0 => {
                errors.push(LineError::new(
                    row.line,
                    "同じバリエーションの組み合わせが重複しています",
                ));
            }
            std::collections::hash_map::Entry::Occupied(_) if index > 0 => {
                errors.push(LineError::new(
                    row.line,
                    "バリエーションのない商品は1行で指定してください",
                ));
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }
    }
    let mut combos: Vec<ComboFormInput> = Vec::new();
    for key in cartesian(&axis_options[..axis_count]) {
        match combo_rows.get(&key) {
            Some(row) => combos.push(combo_input(row, errors, claimed_skus)),
            None => errors.push(LineError::new(
                line,
                format!(
                    "バリエーションの組み合わせ「{}」の行がありません",
                    key.join(" / ")
                ),
            )),
        }
    }

    // Axis form inputs, with stored IDs mapped back by name /
    // value so an update keeps the matrix's identity (and the
    // ordered-combination removal protection sees the same
    // picture the form would send).
    let axes: Vec<AxisFormInput> = (0..axis_count)
        .map(|i| {
            let stored_axis = existing_matrix
                .as_ref()
                .and_then(|m| m.axes.iter().find(|a| a.axis.name == axis_names[i]));
            AxisFormInput {
                id: stored_axis
                    .map(|a| a.axis.id.to_string())
                    .unwrap_or_default(),
                name: axis_names[i].clone(),
                options: axis_options[i]
                    .iter()
                    .map(|value| OptionFormInput {
                        id: stored_axis
                            .and_then(|a| a.options.iter().find(|o| &o.value == value))
                            .map(|o| o.id.to_string())
                            .unwrap_or_default(),
                        value: value.clone(),
                    })
                    .collect(),
            }
        })
        .collect();

    let input = ProductFormInput {
        name: first.get(C_NAME).to_string(),
        description: first.get(C_DESC).to_string(),
        category_id: category_id.map(|id| id.to_string()).unwrap_or_default(),
        price_yen: first.get(C_PRICE).trim().to_string(),
        price_mode: "incl".to_string(),
        price_excl_yen: String::new(),
        cost_yen: first.get(C_COST).trim().to_string(),
        tax_class_id: tax_class_id.map(|id| id.to_string()).unwrap_or_default(),
        sale_price_yen: first.get(C_SALE_PRICE).trim().to_string(),
        sale_starts_at: conv_dt(first.get(C_SALE_START), line, "セール開始", errors),
        sale_ends_at: conv_dt(first.get(C_SALE_END), line, "セール終了", errors),
        product_code: first.get(C_CODE).to_string(),
        brand: first.get(C_BRAND).to_string(),
        manufacturer: first.get(C_MAKER).to_string(),
        country_of_origin: first.get(C_COUNTRY).to_string(),
        spec_text: first.get(C_SPEC).to_string(),
        features_text: first.get(C_FEATURES).to_string(),
        description_sp: first.get(C_DESC_SP).to_string(),
        search_keywords: first.get(C_KEYWORDS).to_string(),
        // Not carried by the CSV map: carried forward from
        // the stored product (defaults on create).
        is_new: existing_product.as_ref().map(|p| p.is_new).unwrap_or(false),
        is_preorder: existing_product
            .as_ref()
            .map(|p| p.is_preorder)
            .unwrap_or(false),
        is_staff_pick: conv_bool(first.get(C_FEATURED), line, "おすすめ", false, errors),
        discount_percent: existing_product
            .as_ref()
            .map(|p| p.discount_percent.to_string())
            .unwrap_or_default(),
        display_order: first.get(C_ORDER).trim().to_string(),
        slug: first.get(C_SLUG).to_string(),
        seo_title: first.get(C_SEO_TITLE).to_string(),
        meta_description: first.get(C_META).to_string(),
        track_stock: conv_bool(first.get(C_TRACK), line, "在庫管理", false, errors),
        allow_backorder: conv_bool(
            first.get(C_BACKORDER),
            line,
            "在庫切れ時販売",
            false,
            errors,
        ),
        low_stock_threshold: first.get(C_LOW_STOCK).trim().to_string(),
        points_earn_percent: first.get(C_POINTS).trim().to_string(),
        weight_grams: first.get(C_WEIGHT).trim().to_string(),
        size_length_mm: first.get(C_SIZE_L).trim().to_string(),
        size_width_mm: first.get(C_SIZE_W).trim().to_string(),
        size_height_mm: first.get(C_SIZE_H).trim().to_string(),
        cool_delivery: conv_bool(first.get(C_COOL), line, "クール便", false, errors),
        mail_delivery: conv_bool(first.get(C_MAIL), line, "メール便", false, errors),
        free_shipping: conv_bool(first.get(C_FREE_SHIP), line, "送料無料対象", false, errors),
        undeliverable_regions: first.get(C_UNDELIVERABLE).to_string(),
        lead_time_days: first.get(C_LEAD).trim().to_string(),
        publish_starts_at: conv_dt(first.get(C_PUB_START), line, "公開開始", errors),
        publish_ends_at: conv_dt(first.get(C_PUB_END), line, "公開終了", errors),
        axes,
        combos,
        // The form path cannot archive; the apply step
        // archives after the write when the CSV asks for it.
        status: if status_token == "public" {
            "public".to_string()
        } else {
            "draft".to_string()
        },
        is_event: existing_product
            .as_ref()
            .map(|p| p.is_event)
            .unwrap_or(false),
        images,
        image_alts,
        tag_ids,
    };

    if !errors.is_empty() {
        return Ok(None);
    }

    let taken =
        crate::repositories::admin::product_repository::taken_identifiers(pool, group.product_id)
            .await?;
    match product_service::validate_product(
        &input,
        &attached,
        valid_tag_ids,
        categories,
        existing_product.as_ref().and_then(|p| p.category_id),
        existing_matrix.as_ref(),
        tax_classes,
        &taken,
    ) {
        Ok(valid) => Ok(Some(ValidGroup {
            product_id: group.product_id,
            valid,
            desired_status: status_token.to_string(),
        })),
        Err(field_errors) => {
            for fe in field_errors {
                errors.push(LineError::new(line, fe.message));
            }
            Ok(None)
        }
    }
}

/// Builds one combination's form input from its CSV row and
/// claims the row's SKU for the file-level uniqueness check.
fn combo_input(
    row: &CsvRow,
    errors: &mut Vec<LineError>,
    claimed_skus: &mut HashMap<String, usize>,
) -> ComboFormInput {
    let sku = row.get(C_SKU).trim();
    if !sku.is_empty() {
        if let Some(first_line) = claimed_skus.get(sku) {
            errors.push(LineError::new(
                row.line,
                format!("SKU「{sku}」が{first_line}行目と重複しています"),
            ));
        } else {
            claimed_skus.insert(sku.to_string(), row.line);
        }
    }
    ComboFormInput {
        stock: row.get(C_STOCK).trim().to_string(),
        price_yen: row.get(C_VAR_PRICE).trim().to_string(),
        sku: row.get(C_SKU).to_string(),
        jan_code: row.get(C_JAN).trim().to_string(),
        image_path: row.get(C_VAR_IMAGE).trim().to_string(),
        is_enabled: conv_bool(
            row.get(C_VAR_ENABLED),
            row.line,
            "バリエーション有効",
            true,
            errors,
        ),
        incoming_qty: row.get(C_INCOMING_QTY).trim().to_string(),
        incoming_date: conv_date(row.get(C_INCOMING_DATE), row.line, "入荷予定日", errors),
    }
}

/// The cartesian product of the axes' option lists, first
/// axis slowest — the form grid's combination order. An
/// axis-less product yields exactly one empty combination.
fn cartesian(axes: &[Vec<String>]) -> Vec<Vec<String>> {
    let mut combos: Vec<Vec<String>> = vec![Vec::new()];
    for options in axes {
        let mut next = Vec::new();
        for prefix in &combos {
            for option in options {
                let mut combo = prefix.clone();
                combo.push(option.clone());
                next.push(combo);
            }
        }
        combos = next;
    }
    combos
}

/// Decodes and stores one upload as a preview batch,
/// returning the batch ID the preview page lives at. A file
/// whose problems are all content problems still becomes a
/// batch (the preview explains them); only transport-level
/// failures (no file, oversize) are refused by the caller.
pub async fn store_upload(
    pool: &PgPool,
    admin_id: i64,
    file_name: &str,
    bytes: &[u8],
) -> Result<i64, AppError> {
    let text = decode_bytes(bytes);
    let (_, report) = validate_upload(pool, &text).await?;
    csv_import_batch_repository::insert_batch(pool, "product", admin_id, file_name, &text, &report)
        .await
}

/// Applies one preview batch: re-validates the stored text
/// against the current database, writes every still-valid
/// product through the form's save path, then flips the
/// batch to applied with its one audit row. Every write —
/// the products, the batch flip and the audit rows —
/// commits or rolls back as ONE transaction: a group
/// failing mid-batch leaves nothing half-applied behind.
/// Returns how many products were created and updated.
pub async fn apply_batch(
    pool: &PgPool,
    admin_id: i64,
    batch_id: i64,
) -> Result<(usize, usize), AppError> {
    let batch = csv_import_batch_repository::find_batch(pool, batch_id)
        .await?
        .filter(|b| b.entity == "product" && b.status == "preview")
        .ok_or(AppError::NotFound)?;
    let (groups, _) = validate_upload(pool, &batch.raw_text).await?;
    // Stock moved by this apply is tagged in the ledger with
    // the batch it came from, so a CSV change is tellable from
    // a form edit in the product's stock history.
    let note = format!("CSV取込 #{batch_id}");
    let (mut created, mut updated) = (0usize, 0usize);
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    for group in groups {
        let id = match group.product_id {
            Some(id) => {
                let existing = product_service::existing_matrix(pool, id).await?;
                product_service::update_product_tx(
                    &mut tx,
                    admin_id,
                    id,
                    &group.valid,
                    &existing,
                    Some(note.as_str()),
                    None,
                )
                .await?;
                updated += 1;
                id
            }
            None => {
                let id = product_service::create_product_tx(
                    &mut tx,
                    admin_id,
                    &group.valid,
                    Some(note.as_str()),
                )
                .await?;
                created += 1;
                id
            }
        };
        if group.desired_status == "archived" {
            product_service::set_status_tx(&mut tx, admin_id, id, "archived", "product.archive")
                .await?;
        }
    }
    csv_import_batch_repository::mark_applied(&mut tx, batch_id).await?;
    crate::services::admin::audit_service::record(
        &mut tx,
        admin_id,
        "product.csv_import",
        "product",
        None,
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok((created, updated))
}

/// Discards one preview batch without touching any product.
pub async fn discard_batch(pool: &PgPool, batch_id: i64) -> Result<(), AppError> {
    let batch = csv_import_batch_repository::find_batch(pool, batch_id)
        .await?
        .filter(|b| b.entity == "product" && b.status == "preview")
        .ok_or(AppError::NotFound)?;
    let _ = batch;
    csv_import_batch_repository::mark_discarded(pool, batch_id).await
}
