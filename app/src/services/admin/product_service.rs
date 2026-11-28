//! Admin product rules.
//!
//! Validation is shared by the create and edit forms and returns
//! every field error at once, in field order, with the exact
//! messages. Two checks are security-relevant: every submitted
//! image path is re-checked on the server — it must name a real
//! file in the uploads store or an image already attached to
//! the product (a crafted POST must not point `image_path` at
//! an arbitrary URL) — and the category
//! ID is re-checked against the managed categories table, so a
//! crafted POST cannot invent a category or park a product in
//! one the operator has retired. Uploaded images are stored
//! under a generated name after a magic-byte type check; the
//! client-supplied filename never reaches a filesystem path.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use sqlx::PgPool;
use uuid::Uuid;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::messages::{
    DATETIME_FORMAT_MESSAGE_BARE, IMAGE_FILE_REQUIRED_MESSAGE, TAX_CLASS_REQUIRED_MESSAGE,
    VARIANT_SPEC_INVALID_MESSAGE,
};
use crate::models::variant::{AxisWithOptions, cartesian, signature_for};
use crate::models::{AdminProduct, AdminProductListRow, ProductCategory, Tag};
use crate::repositories::{
    admin::product_repository as admin_product_repository, category_repository,
    product_image_repository, product_repository, tag_repository, variant_repository,
};
use crate::services::admin::audit_service;
use crate::validation::FieldError;

/// Price ceiling: 99,999,999 yen.
pub const MAX_PRICE_YEN: i64 = 99_999_999;
/// Stock ceiling: 999,999 units.
pub const MAX_STOCK: i64 = 999_999;
/// Variation axes per product ceiling.
pub const MAX_AXES: usize = 5;
/// Options per axis ceiling.
pub const MAX_OPTIONS: usize = 5;
/// Total combinations per product ceiling: the stock grid must
/// stay an editable list, not a phone book.
pub const MAX_COMBOS: usize = 125;

/// Uploaded image ceiling: 20 MB — admins upload full-quality
/// photos. The upload route's body limit sits above this to
/// leave room for the multipart framing.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// The raw product form as submitted, before validation. The
/// controller builds this from the POST body; text fields are
/// kept untrimmed so a failed submit re-renders what was typed.
#[derive(Clone, Default)]
pub struct ProductFormInput {
    /// Product name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Submitted category ID, as typed (the form's select).
    pub category_id: String,
    /// Price, as typed (half-width digits expected).
    pub price_yen: String,
    /// Price entry mode: `excl` enters the tax-exclusive price
    /// in `price_excl_yen` and the server converts; anything
    /// else is the tax-inclusive entry of `price_yen`.
    pub price_mode: String,
    /// Tax-exclusive price as typed, used only in `excl` mode.
    pub price_excl_yen: String,
    /// Cost as typed (admin-only); empty = unrecorded.
    pub cost_yen: String,
    /// Selected tax class ID, as typed.
    pub tax_class_id: String,
    /// Sale price as typed; empty = no sale price.
    pub sale_price_yen: String,
    /// Sale window start (`datetime-local`, JST); empty = open.
    pub sale_starts_at: String,
    /// Sale window end (`datetime-local`, JST); empty = open.
    pub sale_ends_at: String,
    /// Operator-facing product code; empty = unset.
    pub product_code: String,
    /// Brand name.
    pub brand: String,
    /// Manufacturer name.
    pub manufacturer: String,
    /// Country of origin.
    pub country_of_origin: String,
    /// Specification text.
    pub spec_text: String,
    /// Selling-points text.
    pub features_text: String,
    /// Smartphone description override.
    pub description_sp: String,
    /// Extra search keywords.
    pub search_keywords: String,
    /// 新着 badge checkbox.
    pub is_new: bool,
    /// 予約 checkbox.
    pub is_preorder: bool,
    /// おすすめ checkbox.
    pub is_staff_pick: bool,
    /// Label-only promotion percent as typed; empty = 0.
    pub discount_percent: String,
    /// Display order as typed; empty = 0.
    pub display_order: String,
    /// URL slug; empty = unset.
    pub slug: String,
    /// Detail-page title override.
    pub seo_title: String,
    /// Detail-page meta description.
    pub meta_description: String,
    /// Stock tracking checkbox (off = unlimited).
    pub track_stock: bool,
    /// Oversell (backorder) checkbox.
    pub allow_backorder: bool,
    /// Low-stock threshold override as typed; empty = default.
    pub low_stock_threshold: String,
    /// Points earn-rate override as typed; empty = shop rate.
    pub points_earn_percent: String,
    /// Weight in grams as typed; empty = unrecorded.
    pub weight_grams: String,
    /// Parcel length in millimetres as typed.
    pub size_length_mm: String,
    /// Parcel width in millimetres as typed.
    pub size_width_mm: String,
    /// Parcel height in millimetres as typed.
    pub size_height_mm: String,
    /// Chilled delivery checkbox.
    pub cool_delivery: bool,
    /// Mail delivery checkbox.
    pub mail_delivery: bool,
    /// Always-free-shipping checkbox.
    pub free_shipping: bool,
    /// Comma-separated undeliverable prefecture names.
    pub undeliverable_regions: String,
    /// Days until dispatch as typed; empty = standard.
    pub lead_time_days: String,
    /// Publication window start (`datetime-local`, JST).
    pub publish_starts_at: String,
    /// Publication window end (`datetime-local`, JST).
    pub publish_ends_at: String,
    /// Variation axes as submitted, in order.
    pub axes: Vec<AxisFormInput>,
    /// Combination field sets as typed, in the grid's
    /// cartesian order (first axis slowest); one per combo.
    pub combos: Vec<ComboFormInput>,
    /// Status value: `public` or `draft`.
    pub status: String,
    /// Event product checkbox.
    pub is_event: bool,
    /// Submitted image paths in display order (the repeated
    /// `images` field); the first entry is the main image.
    pub images: Vec<String>,
    /// Alt texts aligned with `images` by position (the
    /// repeated `image_alt` field).
    pub image_alts: Vec<String>,
    /// Checked tag IDs, as submitted.
    pub tag_ids: Vec<i64>,
}

/// One axis of the product form as submitted: its stored ID
/// (empty when new), the header text, and its options.
/// One combination row of the form, as posted: the grid's
/// per-combination stock, price override, identifiers, image,
/// enable flag and incoming-stock record.
#[derive(Debug, Clone, Default)]
pub struct ComboFormInput {
    /// Raw stock field (digits).
    pub stock: String,
    /// Raw price override (digits); empty = product price.
    pub price_yen: String,
    /// SKU text; empty = unset.
    pub sku: String,
    /// JAN code (digits); empty = unset.
    pub jan_code: String,
    /// Image path chosen from the product's images; empty = none.
    pub image_path: String,
    /// Whether the combination is enabled (sellable).
    pub is_enabled: bool,
    /// Raw incoming quantity (digits); empty = 0.
    pub incoming_qty: String,
    /// Incoming date (`YYYY-MM-DD`); empty = undecided.
    pub incoming_date: String,
}

#[derive(Clone, Default)]
pub struct AxisFormInput {
    /// Stored axis ID as submitted; empty for a new axis.
    pub id: String,
    /// Header text, as typed.
    pub name: String,
    /// The axis options, in order.
    pub options: Vec<OptionFormInput>,
}

/// One option of the form as submitted.
#[derive(Clone, Default)]
pub struct OptionFormInput {
    /// Stored option ID as submitted; empty for a new option.
    pub id: String,
    /// Option text, as typed.
    pub value: String,
}

/// The validated field set, ready for the repository write.
pub struct ValidProduct {
    /// Trimmed product name.
    pub name: String,
    /// Trimmed description.
    pub description: String,
    /// Category name snapshot, taken from the linked category
    /// record — never from submitted text.
    pub category: String,
    /// The linked category's ID.
    pub category_id: Option<i64>,
    /// Price in integer yen.
    pub price_yen: i64,
    /// Product code; `None` when unset.
    pub product_code: Option<String>,
    /// Brand name.
    pub brand: String,
    /// Manufacturer name.
    pub manufacturer: String,
    /// Country of origin.
    pub country_of_origin: String,
    /// Specification text.
    pub spec_text: String,
    /// Selling-points text.
    pub features_text: String,
    /// Smartphone description override.
    pub description_sp: String,
    /// Extra search keywords.
    pub search_keywords: String,
    /// Tax class ID.
    pub tax_class_id: i64,
    /// Cost in yen; `None` when unrecorded.
    pub cost_yen: Option<i64>,
    /// Sale price in yen; `None` when no sale is set.
    pub sale_price_yen: Option<i64>,
    /// Sale window start.
    pub sale_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Sale window end.
    pub sale_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 新着 badge flag.
    pub is_new: bool,
    /// Label-only promotion percent.
    pub discount_percent: i32,
    /// 予約 flag.
    pub is_preorder: bool,
    /// おすすめ flag.
    pub is_staff_pick: bool,
    /// Default-sort position.
    pub display_order: i32,
    /// Slug; `None` when unset.
    pub slug: Option<String>,
    /// Title override.
    pub seo_title: String,
    /// Meta description.
    pub meta_description: String,
    /// Stock tracking flag.
    pub track_stock: bool,
    /// Oversell flag.
    pub allow_backorder: bool,
    /// Low-stock threshold override.
    pub low_stock_threshold: Option<i32>,
    /// Points earn-rate override; `None` = shop rate.
    pub points_earn_percent: Option<i32>,
    /// Weight in grams.
    pub weight_grams: Option<i32>,
    /// Parcel length in millimetres.
    pub size_length_mm: Option<i32>,
    /// Parcel width in millimetres.
    pub size_width_mm: Option<i32>,
    /// Parcel height in millimetres.
    pub size_height_mm: Option<i32>,
    /// Chilled delivery flag.
    pub cool_delivery: bool,
    /// Mail delivery flag.
    pub mail_delivery: bool,
    /// Free shipping flag.
    pub free_shipping: bool,
    /// Normalized comma-separated undeliverable prefectures.
    pub undeliverable_regions: String,
    /// Days until dispatch.
    pub lead_time_days: Option<i32>,
    /// Publication window start.
    pub publish_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Publication window end.
    pub publish_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The validated variation matrix.
    pub matrix: ValidMatrix,
    /// `public` or `draft`.
    pub status: String,
    /// Event product flag.
    pub is_event: bool,
    /// The product's images in display order as (path, alt
    /// text), each path validated: a real uploads-store file
    /// or an image already attached to the product. Empty
    /// only for drafts — the publish gate refuses a public
    /// product with no images.
    pub images: Vec<(String, String)>,
    /// The synced main-image accessor value: the first
    /// image's path, or `''` when the product has no images.
    /// Written to `products.image_path` in the same
    /// transaction as the image list, so every existing
    /// render site keeps reading the main image from the one
    /// column it already knows.
    pub image_path: String,
    /// Tag IDs (validated against the vocabulary).
    pub tag_ids: Vec<i64>,
}

/// The validated variation matrix: axes and options with their
/// stored IDs (`None` for new rows), plus one stock per
/// combination in cartesian order (first axis slowest) — the
/// order the form's grid posts and the save regenerates.
pub struct ValidMatrix {
    /// Axes in display order.
    pub axes: Vec<ValidAxis>,
    /// One validated field set per combination, cartesian
    /// order.
    pub combos: Vec<ValidCombo>,
}

/// One combination's validated fields — the grid row, ready
/// for the repository's variant field set.
#[derive(Debug, Clone)]
pub struct ValidCombo {
    /// Stock on hand.
    pub stock: i32,
    /// Price override; `None` = the product's effective price.
    pub price_yen: Option<i64>,
    /// SKU; `None` when unset.
    pub sku: Option<String>,
    /// JAN code; `None` when unset.
    pub jan_code: Option<String>,
    /// Image path from the product's images; `None` when unset.
    pub image_path: Option<String>,
    /// Whether the combination is enabled.
    pub is_enabled: bool,
    /// Expected incoming quantity (admin record).
    pub incoming_qty: i32,
    /// Expected incoming date (admin record).
    pub incoming_date: Option<NaiveDate>,
}

impl ValidCombo {
    /// The repository field set for a save.
    fn fields(&self) -> variant_repository::VariantFields {
        variant_repository::VariantFields {
            stock: self.stock,
            price_yen: self.price_yen,
            sku: self.sku.clone(),
            jan_code: self.jan_code.clone(),
            image_path: self.image_path.clone(),
            is_enabled: self.is_enabled,
            incoming_qty: self.incoming_qty,
            incoming_date: self.incoming_date,
        }
    }
}

/// One validated axis.
pub struct ValidAxis {
    /// Stored axis ID; `None` for a new axis.
    pub id: Option<i64>,
    /// Trimmed header text.
    pub name: String,
    /// Options in display order.
    pub options: Vec<ValidOption>,
}

/// One validated option.
pub struct ValidOption {
    /// Stored option ID; `None` for a new option.
    pub id: Option<i64>,
    /// Trimmed option text.
    pub value: String,
}

/// One existing combination as the matrix validation sees it:
/// its option set and whether any order references it (the D5
/// protection).
pub struct ExistingVariant {
    /// Variant ID.
    pub id: i64,
    /// Stored stock, for the manual-adjustment ledger delta.
    pub stock: i32,
    /// Its option IDs, sorted for set comparison.
    pub option_ids: Vec<i64>,
    /// Whether any order line references this variant.
    pub ordered: bool,
}

/// The stored matrix of a product being edited: its axes with
/// options and its existing combinations. Create validates
/// against `None` (nothing stored, nothing ordered).
pub struct ExistingMatrix {
    /// Axes with options, in display order.
    pub axes: Vec<AxisWithOptions>,
    /// Existing combinations.
    pub variants: Vec<ExistingVariant>,
}

/// One freshly stored upload: the generated filename and
/// the served path the upload endpoint answers with.
pub struct UploadedImage {
    /// Bare generated filename, e.g. `<uuid>.png`.
    pub filename: String,
    /// Served path, e.g. `/static/images/uploads/<uuid>.png`.
    pub path: String,
}

/// The uploads-store prefix every newly attached image path
/// carries. Attached images from before the gallery redesign
/// (the built-in assets) keep their own prefix and validate
/// through the attached list instead — never through this rule.
const UPLOADS_PREFIX: &str = "/static/images/uploads/";

/// Whether a submitted image path names a real file in the
/// uploads store. The name must be a bare generated filename —
/// no directory part, a raster extension the upload validation
/// accepts — and the file must exist: a crafted POST can
/// neither invent a path nor escape the uploads directory.
/// Shared with the home hero picker, which attaches images
/// from the same store under the same rule.
pub(crate) fn is_upload_path(path: &str) -> bool {
    let Some(name) = path.strip_prefix(UPLOADS_PREFIX) else {
        return false;
    };
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return false;
    }
    let is_raster = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["png", "jpg", "webp", "gif"].contains(&e));
    is_raster
        && crate::routes::static_dir()
            .join("images")
            .join("uploads")
            .join(name)
            .is_file()
}

/// Returns a product's attached image paths in display order
/// (main first), for the edit form's gallery prefill and the
/// save validation's attached list.
pub async fn product_images(pool: &PgPool, product_id: i64) -> Result<Vec<String>, AppError> {
    product_image_repository::list_for_product(pool, product_id).await
}

/// Why storing an uploaded image failed: a refusal whose
/// message the admin can act on, or an internal failure whose
/// detail was logged where it happened.
pub enum UploadError {
    /// The upload itself was refused (size or type); the
    /// message is shown to the admin as-is.
    Refused(String),
    /// A storage or audit failure; details are in the log.
    Internal,
}

/// Stores one uploaded product image and returns the
/// generated filename and the path the storefront serves.
///
/// The type is read from the file's magic bytes, never from
/// the client-supplied name or content-type header: PNG,
/// JPEG, WebP and GIF rasters only — SVG is refused, because
/// it is active content that would be served from the store's
/// own origin. The stored name is a fresh UUID with the
/// canonical extension for the detected type, prefixed
/// with the first five kept characters of the client's
/// file stem so the admin can recognise the image in
/// pickers (see stored_image_filename below). The audit
/// row commits after the write; if recording fails, the file
/// is removed again — there is no unrecorded mutation path.
pub async fn store_uploaded_image(
    pool: &PgPool,
    admin_id: i64,
    original_name: Option<&str>,
    bytes: &[u8],
) -> Result<UploadedImage, UploadError> {
    if bytes.is_empty() {
        return Err(UploadError::Refused(
            IMAGE_FILE_REQUIRED_MESSAGE.to_string(),
        ));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(UploadError::Refused(
            "画像は20MB以下にしてください".to_string(),
        ));
    }
    // Type comes from the magic bytes alone: a renamed file
    // keeps the type its bytes declare, and anything that is
    // not one of the four rasters is refused here.
    let Some(ext) = uploaded_image_extension(bytes) else {
        return Err(UploadError::Refused(
            "対応していない画像形式です（PNG / JPEG / WebP / GIF のみアップロードできます）"
                .to_string(),
        ));
    };
    let filename = stored_image_filename(original_name, ext);
    let uploads = crate::routes::static_dir().join("images").join("uploads");
    if let Err(e) = tokio::fs::create_dir_all(&uploads).await {
        tracing::error!("create uploads dir: {e}");
        return Err(UploadError::Internal);
    }
    // The UUID in the name is fresh, so no reader can know
    // the full path before this function returns it: a direct
    // write cannot be observed half-finished.
    let stored = uploads.join(&filename);
    if let Err(e) = tokio::fs::write(&stored, bytes).await {
        tracing::error!("write uploaded image: {e}");
        return Err(UploadError::Internal);
    }
    let recorded = async {
        let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
        audit_service::record(
            &mut tx,
            admin_id,
            "product.image_upload",
            "product_image",
            None,
        )
        .await?;
        tx.commit().await.map_err(|e| AppError::db("commit", e))?;
        Ok::<(), AppError>(())
    }
    .await;
    if let Err(e) = recorded {
        tracing::error!("audit uploaded image: {e}");
        let _ = tokio::fs::remove_file(&stored).await;
        return Err(UploadError::Internal);
    }
    // Derivatives are best-effort: the resized WebP
    // copies the storefront serves are generated off the async
    // workers, and a failure is only logged — the original is
    // stored, and every surface falls back to it when a
    // derivative is missing. GIFs are skipped: a still copy
    // would freeze the animation.
    if ext != "gif" {
        let stem = filename
            .rsplit_once('.')
            .map_or(filename.as_str(), |(stem, _)| stem)
            .to_string();
        let job = {
            let bytes = bytes.to_vec();
            let dir = uploads.clone();
            tokio::task::spawn_blocking(move || {
                generate_derivatives(&bytes, &dir, &stem, &DerivativeSpec::from_config())
            })
        };
        match job.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("image derivatives for {filename}: {e}"),
            Err(e) => tracing::warn!("image derivative task for {filename}: {e}"),
        }
    }
    Ok(UploadedImage {
        path: format!("/static/images/uploads/{filename}"),
        filename,
    })
}

/// How the derivatives for one upload are generated
///: the two longest-edge caps and the JPEG quality.
pub struct DerivativeSpec {
    /// Longest edge of the thumbnail derivative, in pixels;
    /// zero skips the derivative.
    pub thumb_max_px: u32,
    /// Longest edge of the medium derivative, in pixels;
    /// zero skips the derivative.
    pub medium_max_px: u32,
    /// JPEG quality (1-100) for derivatives of JPEG sources.
    pub jpeg_quality: u8,
}

impl DerivativeSpec {
    /// The spec from the typed configuration.
    fn from_config() -> Self {
        Self {
            thumb_max_px: CONFIG.image_thumb_max_px,
            medium_max_px: CONFIG.image_medium_max_px,
            jpeg_quality: CONFIG.image_jpeg_quality,
        }
    }
}

/// Generates the resized derivatives for one stored upload
///: `<stem>_thumb.<dext>` and `<stem>_medium.<dext>`
/// beside the original in the uploads directory, each
/// capped at the spec's longest edge and never upscaled.
/// The encoding follows the source's family (the naming
/// rule lives in [`crate::util::image_paths`]): a JPEG source is
/// re-encoded as JPEG at the spec's quality — a lossless
/// copy of a photo stays needlessly large — while the
/// other rasters become lossless WebP, so transparency
/// survives. Synchronous CPU work — call it inside
/// `spawn_blocking`, never on an async worker. An error
/// names the step that failed; the caller logs it and
/// keeps the upload.
pub fn generate_derivatives(
    bytes: &[u8],
    uploads_dir: &std::path::Path,
    stem: &str,
    spec: &DerivativeSpec,
) -> Result<(), String> {
    let format = image::guess_format(bytes).map_err(|e| format!("sniff format failed: {e}"))?;
    let dext = crate::util::image_paths::derivative_ext_for(match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::WebP => "webp",
        _ => return Err(format!("unsupported source format: {format:?}")),
    });
    let img = image::load_from_memory(bytes).map_err(|e| format!("decode failed: {e}"))?;
    for (suffix, max_px) in [
        (crate::util::image_paths::THUMB_SUFFIX, spec.thumb_max_px),
        (crate::util::image_paths::MEDIUM_SUFFIX, spec.medium_max_px),
    ] {
        if max_px == 0 {
            continue;
        }
        let resized = resize_to_cap(&img, max_px);
        let mut buf = Vec::new();
        if format == image::ImageFormat::Jpeg {
            let encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, spec.jpeg_quality);
            resized
                .write_with_encoder(encoder)
                .map_err(|e| format!("encode {suffix} failed: {e}"))?;
        } else {
            resized
                .write_to(
                    &mut std::io::Cursor::new(&mut buf),
                    image::ImageFormat::WebP,
                )
                .map_err(|e| format!("encode {suffix} failed: {e}"))?;
        }
        let name = format!("{stem}{suffix}.{dext}");
        std::fs::write(uploads_dir.join(&name), &buf)
            .map_err(|e| format!("write {name} failed: {e}"))?;
    }
    Ok(())
}

/// The image scaled so its longest edge is at most
/// `max_px`, or the image itself when it already fits:
/// derivatives never upscale. A fitting image is borrowed,
/// not cloned - the pixel buffer is the largest allocation
/// in the derivative pass, and encoding only reads it.
fn resize_to_cap<'a>(
    img: &'a image::DynamicImage,
    max_px: u32,
) -> std::borrow::Cow<'a, image::DynamicImage> {
    if img.width() <= max_px && img.height() <= max_px {
        return std::borrow::Cow::Borrowed(img);
    }
    std::borrow::Cow::Owned(img.resize(max_px, max_px, image::imageops::FilterType::Lanczos3))
}

/// Builds the stored filename for an uploaded image: the
/// recognisable prefix from the client's original
/// filename, a fresh UUID, and the canonical extension
/// for the detected type.
///
/// The prefix is the first five kept characters of the
/// original file stem, where kept means alphanumeric or
/// `-`/`_` (a leading `-`/`_` run is dropped, so the name
/// never starts with one). Separators, dots, spaces and
/// every other character are dropped, so the prefix is
/// always one safe path component and can never smuggle
/// in an extension of its own. A name with no kept
/// characters (or no name at all) yields the bare UUID
/// form used before prefixes existed.
fn stored_image_filename(original_name: Option<&str>, ext: &str) -> String {
    let kept: String = original_name
        .and_then(|name| std::path::Path::new(name).file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(5)
        .collect();
    let prefix = kept.trim_start_matches(['-', '_']);
    let uuid = Uuid::new_v4();
    if prefix.is_empty() {
        format!("{uuid}.{ext}")
    } else {
        format!("{prefix}-{uuid}.{ext}")
    }
}

/// Returns the canonical extension for an uploaded image's
/// detected type, read from its magic bytes: PNG, JPEG, WebP
/// or GIF. `None` for anything else — including SVG, which is
/// markup, not a raster, and is never accepted as an upload.
fn uploaded_image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes[8..12].starts_with(b"WEBP") {
        Some("webp")
    } else {
        None
    }
}

/// The repository's list-query shape, re-exported so the list
/// controller builds it through this service and never names the
/// repository (controllers → services → repositories).
pub use crate::repositories::admin::product_repository::AdminProductQuery;

/// Returns one page of the admin product list plus the total
/// count of rows matching the same filters.
pub async fn list_products(
    pool: &PgPool,
    query: &AdminProductQuery<'_>,
) -> Result<(Vec<AdminProduct>, i64), AppError> {
    admin_product_repository::list_products(pool, query).await
}

/// The product list page's rows: the narrow list shape —
/// the long text columns are not read. Same filters and
/// pagination as [`list_products`].
pub async fn list_products_page(
    pool: &PgPool,
    query: &AdminProductQuery<'_>,
) -> Result<(Vec<AdminProductListRow>, i64), AppError> {
    admin_product_repository::list_products_page(pool, query).await
}

/// Per-tab counts for the product list's tab labels:
/// (all-excluding-archived, public, draft, archived).
pub async fn tab_counts(pool: &PgPool) -> Result<(i64, i64, i64, i64), AppError> {
    admin_product_repository::tab_counts(pool).await
}

/// Finds one product in any publication state, for the admin edit
/// screen and the archive actions' existence check.
pub async fn find_product(pool: &PgPool, id: i64) -> Result<Option<AdminProduct>, AppError> {
    admin_product_repository::find_product(pool, id).await
}

/// Returns the whole tag vocabulary, in seed order — the form's
/// checkbox list, the create/edit validation vocabulary and the
/// list filter's options.
pub async fn list_tags(pool: &PgPool) -> Result<Vec<Tag>, AppError> {
    tag_repository::list_tags(pool).await
}

/// Returns the tag IDs currently attached to one product.
pub async fn product_tag_ids(pool: &PgPool, product_id: i64) -> Result<Vec<i64>, AppError> {
    tag_repository::product_tag_ids(pool, product_id).await
}

/// Returns every managed category in display order — the form's
/// select options and the category validation vocabulary.
pub async fn list_category_records(pool: &PgPool) -> Result<Vec<ProductCategory>, AppError> {
    category_repository::list_all(pool).await
}

/// Returns every category in use, for the product list's filter
/// and the form's suggestions. Reads through the storefront
/// repository (categories are one shared vocabulary); the error
/// is wrapped here with the context the admin pages log it under.
pub async fn list_categories(pool: &PgPool) -> Result<Vec<String>, AppError> {
    product_repository::find_categories_all(pool)
        .await
        .map_err(|e| AppError::db("admin product categories", e))
}

/// Returns every product's ID and name, for filter selects (the
/// review moderation filter includes archived products).
pub async fn list_all_id_name(pool: &PgPool) -> Result<Vec<(i64, String)>, AppError> {
    admin_product_repository::list_all_id_name(pool).await
}

/// Validates the product form, returning every error at once.
/// `attached` is the product's currently attached image paths
/// (empty on create): a submitted image must be one of those
/// or a real uploads-store file. `valid_tag_ids` is the tag
/// vocabulary.
/// `categories` is the managed category list and
/// `current_category_id` the product's own category on edit
/// (`None` on create): an inactive category validates only as
/// the product's own, so an unrelated edit never forces a
/// category change. `existing` is the stored variation matrix
/// on edit (`None` on create): the removal protection
/// validates against it.
#[allow(clippy::too_many_arguments)]
pub fn validate_product(
    input: &ProductFormInput,
    attached: &[String],
    valid_tag_ids: &[i64],
    categories: &[ProductCategory],
    current_category_id: Option<i64>,
    existing: Option<&ExistingMatrix>,
    tax_classes: &[crate::models::tax::ClassRate],
    taken: &admin_product_repository::TakenIdentifiers,
) -> Result<ValidProduct, Vec<FieldError>> {
    let mut errors: Vec<FieldError> = Vec::new();

    let name = input.name.trim().to_string();
    if name.is_empty() {
        errors.push(FieldError::new("name", "商品名を入力してください"));
    } else if name.chars().count() > 100 {
        errors.push(FieldError::new(
            "name",
            "商品名は100文字以内で入力してください",
        ));
    }

    let description = input.description.trim().to_string();
    if description.chars().count() > 2000 {
        errors.push(FieldError::new(
            "description",
            "説明は2,000文字以内で入力してください",
        ));
    }

    let category = validate_category_choice(input, categories, current_category_id, &mut errors);
    let (tax_class_id, tax_rate) = validate_tax_class_choice(input, tax_classes, &mut errors);
    let PricingFields {
        price,
        cost_yen,
        sale_price_yen,
        sale_starts_at,
        sale_ends_at,
        publish_starts_at,
        publish_ends_at,
    } = validate_pricing(input, tax_rate, &mut errors);
    let TextFields {
        product_code,
        brand,
        manufacturer,
        country_of_origin,
        spec_text,
        features_text,
        description_sp,
        search_keywords,
        seo_title,
        meta_description,
        slug,
    } = validate_text_fields(input, taken, &mut errors);
    let NumericFields {
        discount_percent,
        display_order,
        low_stock_threshold,
        points_earn_percent,
        weight_grams,
        size_length_mm,
        size_width_mm,
        size_height_mm,
        lead_time_days,
    } = validate_numeric_options(input, &mut errors);

    // Undeliverable regions: comma-separated prefecture names,
    // each checked against the address code's prefecture list.
    let mut regions: Vec<String> = Vec::new();
    for token in input.undeliverable_regions.split([',', '、']) {
        let name = token.trim();
        if name.is_empty() {
            continue;
        }
        if !crate::views::storefront::PREFECTURES.contains(&name) {
            errors.push(FieldError::new(
                "undeliverable_regions",
                &format!("都道府県名が正しくありません: {name}"),
            ));
        } else if !regions.iter().any(|r| r == name) {
            regions.push(name.to_string());
        }
    }
    let undeliverable_regions = regions.join(",");

    // The images a combination may point at: anything already
    // attached or submitted with this save.
    let image_candidates: Vec<String> = attached
        .iter()
        .cloned()
        .chain(input.images.iter().map(|p| p.trim().to_string()))
        .filter(|p| !p.is_empty())
        .collect();
    let matrix = validate_matrix(input, existing, taken, &image_candidates, &mut errors);

    // Status comes from a fixed select; anything else is a
    // crafted value and is refused by falling back to draft —
    // the safe state — rather than trusting the input.
    let status = if input.status == "public" {
        "public"
    } else {
        "draft"
    }
    .to_string();

    // Images: the submitted order is the gallery order and
    // the first entry is the main image. Every path must name
    // a real uploads-store file or an image already attached
    // to this product — a crafted POST naming an external URL,
    // a built-in asset for a product that does not carry it,
    // or a traversal path is refused. There is no count cap.
    // The publish gate sits on the same field:
    // the RESULTING state may be public only with at least one
    // image, while a draft saves freely with none — one rule
    // covers create-as-public and removing the last image of
    // a public product alike.
    let mut images: Vec<(String, String)> = Vec::new();
    let mut image_invalid = false;
    let mut image_duplicated = false;
    for (index, raw) in input.images.iter().enumerate() {
        let path = raw.trim();
        if path.is_empty() {
            continue;
        }
        if images.iter().any(|(p, _)| p == path) {
            image_duplicated = true;
            continue;
        }
        if !attached.iter().any(|p| p == path) && !is_upload_path(path) {
            image_invalid = true;
            continue;
        }
        let alt = input
            .image_alts
            .get(index)
            .map(|a| a.trim().to_string())
            .unwrap_or_default();
        if alt.chars().count() > 200 {
            errors.push(FieldError::new(
                "image",
                "画像の代替テキストは200文字以内で入力してください",
            ));
            continue;
        }
        images.push((path.to_string(), alt));
    }
    if image_invalid {
        errors.push(FieldError::new("image", "使用できない画像が含まれています"));
    }
    if image_duplicated {
        errors.push(FieldError::new("image", "同じ画像が複数含まれています"));
    }
    if status == "public" && images.is_empty() {
        errors.push(FieldError::new(
            "image",
            "公開するには画像を1枚以上登録してください",
        ));
    }

    let tag_ids: Vec<i64> = input
        .tag_ids
        .iter()
        .copied()
        .filter(|id| valid_tag_ids.contains(id))
        .collect();

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(ValidProduct {
        name,
        description,
        category: category.map(|c| c.name.clone()).unwrap_or_default(),
        category_id: category.map(|c| c.id),
        price_yen: price.unwrap_or_default(),
        product_code,
        brand,
        manufacturer,
        country_of_origin,
        spec_text,
        features_text,
        description_sp,
        search_keywords,
        tax_class_id,
        cost_yen: cost_yen.flatten(),
        sale_price_yen,
        sale_starts_at: sale_starts_at.flatten(),
        sale_ends_at: sale_ends_at.flatten(),
        is_new: input.is_new,
        discount_percent,
        is_preorder: input.is_preorder,
        is_staff_pick: input.is_staff_pick,
        display_order,
        slug,
        seo_title,
        meta_description,
        track_stock: input.track_stock,
        allow_backorder: input.allow_backorder,
        low_stock_threshold,
        points_earn_percent,
        weight_grams,
        size_length_mm,
        size_width_mm,
        size_height_mm,
        cool_delivery: input.cool_delivery,
        mail_delivery: input.mail_delivery,
        free_shipping: input.free_shipping,
        undeliverable_regions,
        lead_time_days,
        publish_starts_at: publish_starts_at.flatten(),
        publish_ends_at: publish_ends_at.flatten(),
        matrix: matrix.expect("the matrix validates when no errors were pushed"),
        status,
        is_event: input.is_event,
        images: images.clone(),
        image_path: images
            .first()
            .map(|(path, _)| path.clone())
            .unwrap_or_default(),
        tag_ids,
    })
}

/// One amount field's validation rule: its name, its three
/// spec messages, and its inclusive range.
struct AmountRule {
    /// Form field name the errors attach to.
    field: &'static str,
    /// Message when the field is empty.
    empty_msg: &'static str,
    /// Message when the value is not a valid in-range integer.
    integer_msg: &'static str,
    /// Message when the value exceeds the ceiling.
    max_msg: &'static str,
    /// Smallest accepted value.
    min: i64,
    /// Largest accepted value.
    max: i64,
}

/// The category choice of the product form, extracted from
/// [`validate_product`]: resolves the submitted ID to a
/// managed category and records the field error when the
/// choice names no category or an inactive one that is not
/// the product's own.
fn validate_category_choice<'a>(
    input: &ProductFormInput,
    categories: &'a [ProductCategory],
    current_category_id: Option<i64>,
    errors: &mut Vec<FieldError>,
) -> Option<&'a ProductCategory> {
    // The category is a managed record now, not free text: the
    // submitted ID must name a real category, and an inactive
    // category is accepted only when it is the product's own
    // current one (the edit form shows it, marked inactive, so
    // an unrelated edit never forces a category change). The
    // stored name is the record's name — the snapshot the
    // storefront search reads — never the submitted text.
    let category = input
        .category_id
        .trim()
        .parse::<i64>()
        .ok()
        .and_then(|id| categories.iter().find(|c| c.id == id));
    match category {
        None => errors.push(FieldError::new("category", "カテゴリーを選択してください")),
        Some(c) if !c.is_active && current_category_id != Some(c.id) => {
            errors.push(FieldError::new(
                "category",
                "選択されたカテゴリーは現在無効です",
            ));
        }
        _ => {}
    }
    category
}

/// The tax class choice of the product form, extracted from
/// [`validate_product`]: the chosen class's ID and the rate
/// effective today; an unchosen class records the field
/// error and falls back to the default class and rate so
/// the price conversion below still runs.
fn validate_tax_class_choice(
    input: &ProductFormInput,
    tax_classes: &[crate::models::tax::ClassRate],
    errors: &mut Vec<FieldError>,
) -> (i64, i32) {
    // The tax class drives both the stored classification and,
    // in 税抜 entry mode, the conversion rate (the rate
    // effective today for the chosen class).
    let tax_class = input
        .tax_class_id
        .trim()
        .parse::<i64>()
        .ok()
        .and_then(|id| tax_classes.iter().find(|cr| cr.class.id == id));
    let (tax_class_id, tax_rate) = match tax_class {
        Some(cr) => (cr.class.id, cr.rate_percent),
        None => {
            errors.push(FieldError::new("tax_class_id", TAX_CLASS_REQUIRED_MESSAGE));
            (1, 10)
        }
    };
    (tax_class_id, tax_rate)
}

/// The price-shaped results of [`validate_product`]'s
/// pricing section.
struct PricingFields {
    /// The stored, always tax-inclusive, price.
    price: Option<i64>,
    /// The optional cost price: the outer Option is the
    /// parse outcome, the inner the value (empty = unset) —
    /// the same nesting the field parsers return.
    cost_yen: Option<Option<i64>>,
    /// The optional sale price: the pricing section's own
    /// cross-check has already flattened it (a sale price
    /// exists only alongside a valid regular price).
    sale_price_yen: Option<i64>,
    /// The sale window's start; nested like `cost_yen`.
    sale_starts_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// The sale window's end; nested like `cost_yen`.
    sale_ends_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// The publish window's start; nested like `cost_yen`.
    publish_starts_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// The publish window's end; nested like `cost_yen`.
    publish_ends_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

/// The pricing section of the product form, extracted from
/// [`validate_product`]: the price in both entry modes, the
/// optional cost and sale prices with their cross-checks,
/// and the sale and publish windows.
fn validate_pricing(
    input: &ProductFormInput,
    tax_rate: i32,
    errors: &mut Vec<FieldError>,
) -> PricingFields {
    // The stored price is always tax-INCLUSIVE. In 税抜 entry
    // mode the typed figure is exclusive and the server adds
    // the class's tax with the shop's rounding rule; the
    // converted result must satisfy the same range.
    let price = if input.price_mode == "excl" {
        let excl = parse_amount(
            &input.price_excl_yen,
            &AmountRule {
                field: "price_excl_yen",
                empty_msg: "税抜価格を入力してください",
                integer_msg: "税抜価格は1円以上の整数で入力してください",
                max_msg: "税抜価格が大きすぎます",
                min: 1,
                max: MAX_PRICE_YEN,
            },
            errors,
        );
        match excl {
            Some(excl) => {
                let incl = crate::models::tax::inclusive_from_exclusive(
                    excl,
                    tax_rate,
                    crate::models::tax::TaxRounding::configured(),
                );
                if incl > MAX_PRICE_YEN {
                    errors.push(FieldError::new(
                        "price_excl_yen",
                        "税込価格は99,999,999円以下で入力してください",
                    ));
                    None
                } else {
                    Some(incl)
                }
            }
            None => None,
        }
    } else {
        parse_amount(
            &input.price_yen,
            &AmountRule {
                field: "price_yen",
                empty_msg: "価格を入力してください",
                integer_msg: "価格は1円以上の整数で入力してください",
                max_msg: "価格は99,999,999円以下で入力してください",
                min: 1,
                max: MAX_PRICE_YEN,
            },
            errors,
        )
    };

    // Optional amounts share one shape: empty means unset.
    let cost_yen = parse_optional_amount(
        &input.cost_yen,
        "cost_yen",
        "原価は0円以上の整数で入力してください",
        errors,
    );
    let sale_price_yen = parse_optional_amount(
        &input.sale_price_yen,
        "sale_price_yen",
        "セール価格は1円以上の整数で入力してください",
        errors,
    );
    let sale_price_yen = match (sale_price_yen, price) {
        (Some(Some(sale)), Some(regular)) if sale >= regular => {
            errors.push(FieldError::new(
                "sale_price_yen",
                "セール価格は通常価格より低く設定してください",
            ));
            Some(sale)
        }
        (other, _) => other.flatten(),
    };
    let sale_starts_at = parse_jst_local(&input.sale_starts_at, "sale_starts_at", errors);
    let sale_ends_at = parse_jst_local(&input.sale_ends_at, "sale_ends_at", errors);
    if let (Some(Some(start)), Some(Some(end))) = (sale_starts_at, sale_ends_at)
        && start >= end
    {
        errors.push(FieldError::new(
            "sale_ends_at",
            "セール終了日時は開始日時より後にしてください",
        ));
    }
    let publish_starts_at = parse_jst_local(&input.publish_starts_at, "publish_starts_at", errors);
    let publish_ends_at = parse_jst_local(&input.publish_ends_at, "publish_ends_at", errors);
    if let (Some(Some(start)), Some(Some(end))) = (publish_starts_at, publish_ends_at)
        && start >= end
    {
        errors.push(FieldError::new(
            "publish_ends_at",
            "公開終了日時は開始日時より後にしてください",
        ));
    }
    PricingFields {
        price,
        cost_yen,
        sale_price_yen,
        sale_starts_at,
        sale_ends_at,
        publish_starts_at,
        publish_ends_at,
    }
}

/// The text-shaped results of [`validate_product`]'s text
/// section.
struct TextFields {
    /// The optional product code.
    product_code: Option<String>,
    /// The brand text.
    brand: String,
    /// The manufacturer text.
    manufacturer: String,
    /// The country-of-origin text.
    country_of_origin: String,
    /// The specification text.
    spec_text: String,
    /// The features text.
    features_text: String,
    /// The smartphone description.
    description_sp: String,
    /// The search keywords.
    search_keywords: String,
    /// The SEO title.
    seo_title: String,
    /// The meta description.
    meta_description: String,
    /// The optional URL slug, format- and uniqueness-checked.
    slug: Option<String>,
}

/// The text section of the product form, extracted from
/// [`validate_product`]: every length-capped text field and
/// the slug, with its format and uniqueness checks.
fn validate_text_fields(
    input: &ProductFormInput,
    taken: &admin_product_repository::TakenIdentifiers,
    errors: &mut Vec<FieldError>,
) -> TextFields {
    // Texts with per-field ceilings.
    let product_code = optional_text(
        &input.product_code,
        64,
        "product_code",
        "商品コードは64文字以内で入力してください",
        errors,
    );
    if let Some(code) = &product_code
        && taken.product_codes.iter().any(|c| c == code)
    {
        errors.push(FieldError::new(
            "product_code",
            "この商品コードは既に使用されています",
        ));
    }
    let brand = required_text(
        &input.brand,
        100,
        "brand",
        "ブランドは100文字以内で入力してください",
        errors,
    );
    let manufacturer = required_text(
        &input.manufacturer,
        100,
        "manufacturer",
        "メーカーは100文字以内で入力してください",
        errors,
    );
    let country_of_origin = required_text(
        &input.country_of_origin,
        100,
        "country_of_origin",
        "原産国は100文字以内で入力してください",
        errors,
    );
    let spec_text = required_text(
        &input.spec_text,
        5000,
        "spec_text",
        "仕様は5,000文字以内で入力してください",
        errors,
    );
    let features_text = required_text(
        &input.features_text,
        5000,
        "features_text",
        "商品の特徴は5,000文字以内で入力してください",
        errors,
    );
    let description_sp = required_text(
        &input.description_sp,
        2000,
        "description_sp",
        "スマートフォン向け説明は2,000文字以内で入力してください",
        errors,
    );
    let search_keywords = required_text(
        &input.search_keywords,
        500,
        "search_keywords",
        "検索キーワードは500文字以内で入力してください",
        errors,
    );
    let seo_title = required_text(
        &input.seo_title,
        200,
        "seo_title",
        "SEOタイトルは200文字以内で入力してください",
        errors,
    );
    let meta_description = required_text(
        &input.meta_description,
        500,
        "meta_description",
        "メタディスクリプションは500文字以内で入力してください",
        errors,
    );

    let slug = optional_text(
        &input.slug,
        50,
        "slug",
        "スラッグは50文字以内で入力してください",
        errors,
    );
    if let Some(s) = &slug {
        if !crate::validation::is_valid_slug(s) {
            errors.push(FieldError::new(
                "slug",
                "スラッグは半角小文字・数字・ハイフンで入力してください",
            ));
        } else if taken.slugs.iter().any(|x| x == s) {
            errors.push(FieldError::new(
                "slug",
                "このスラッグは既に使用されています",
            ));
        }
    }
    TextFields {
        product_code,
        brand,
        manufacturer,
        country_of_origin,
        spec_text,
        features_text,
        description_sp,
        search_keywords,
        seo_title,
        meta_description,
        slug,
    }
}

/// The numeric results of [`validate_product`]'s options
/// section.
struct NumericFields {
    /// The manual discount percentage, 0 when unset.
    discount_percent: i32,
    /// The display order, 0 when unset.
    display_order: i32,
    /// The low-stock threshold, when set.
    low_stock_threshold: Option<i32>,
    /// The points earn-rate override, when set.
    points_earn_percent: Option<i32>,
    /// The weight in grams, when set.
    weight_grams: Option<i32>,
    /// The length in millimetres, when set.
    size_length_mm: Option<i32>,
    /// The width in millimetres, when set.
    size_width_mm: Option<i32>,
    /// The height in millimetres, when set.
    size_height_mm: Option<i32>,
    /// The lead time in days, when set.
    lead_time_days: Option<i32>,
}

/// The numeric options section of the product form,
/// extracted from [`validate_product`]: each optional
/// integer parsed against its own range and message.
fn validate_numeric_options(
    input: &ProductFormInput,
    errors: &mut Vec<FieldError>,
) -> NumericFields {
    let discount_percent = parse_optional_int(
        &input.discount_percent,
        "discount_percent",
        0,
        100,
        "割引率は0〜100の整数で入力してください",
        errors,
    )
    .unwrap_or(0) as i32;
    let display_order = parse_optional_int(
        &input.display_order,
        "display_order",
        -99999,
        99999,
        "表示順は整数で入力してください",
        errors,
    )
    .unwrap_or(0) as i32;
    let low_stock_threshold = parse_optional_int(
        &input.low_stock_threshold,
        "low_stock_threshold",
        0,
        9999,
        "在庫わずか閾値は0以上の整数で入力してください",
        errors,
    )
    .map(|v| v as i32);
    let points_earn_percent = parse_optional_int(
        &input.points_earn_percent,
        "points_earn_percent",
        0,
        100,
        "ポイント付与率は0〜100の整数で入力してください",
        errors,
    )
    .map(|v| v as i32);
    let weight_grams = parse_optional_int(
        &input.weight_grams,
        "weight_grams",
        0,
        1_000_000,
        "重量は0以上の整数（g）で入力してください",
        errors,
    )
    .map(|v| v as i32);
    let size_length_mm = parse_optional_int(
        &input.size_length_mm,
        "size_length_mm",
        0,
        100_000,
        "長さは0以上の整数（mm）で入力してください",
        errors,
    )
    .map(|v| v as i32);
    let size_width_mm = parse_optional_int(
        &input.size_width_mm,
        "size_width_mm",
        0,
        100_000,
        "幅は0以上の整数（mm）で入力してください",
        errors,
    )
    .map(|v| v as i32);
    let size_height_mm = parse_optional_int(
        &input.size_height_mm,
        "size_height_mm",
        0,
        100_000,
        "高さは0以上の整数（mm）で入力してください",
        errors,
    )
    .map(|v| v as i32);
    let lead_time_days = parse_optional_int(
        &input.lead_time_days,
        "lead_time_days",
        0,
        365,
        "発送予定日数は0〜365の整数で入力してください",
        errors,
    )
    .map(|v| v as i32);
    NumericFields {
        discount_percent,
        display_order,
        low_stock_threshold,
        points_earn_percent,
        weight_grams,
        size_length_mm,
        size_width_mm,
        size_height_mm,
        lead_time_days,
    }
}

/// Parses one half-width-digits amount field under its rule.
/// Returns `None` after pushing the fitting error: empty →
/// required message; non-digits or below the minimum → the
/// integer message; above the maximum → the ceiling message.
/// A digit string too long for i64 lands on the ceiling
/// message, which is the honest one.
fn parse_amount(raw: &str, rule: &AmountRule, errors: &mut Vec<FieldError>) -> Option<i64> {
    let text = raw.trim();
    if text.is_empty() {
        errors.push(FieldError::new(rule.field, rule.empty_msg));
        return None;
    }
    if !text.chars().all(|c| c.is_ascii_digit()) {
        errors.push(FieldError::new(rule.field, rule.integer_msg));
        return None;
    }
    match text.parse::<i64>() {
        Ok(v) if v < rule.min => {
            errors.push(FieldError::new(rule.field, rule.integer_msg));
            None
        }
        Ok(v) if v > rule.max => {
            errors.push(FieldError::new(rule.field, rule.max_msg));
            None
        }
        Ok(v) => Some(v),
        Err(_) => {
            errors.push(FieldError::new(rule.field, rule.max_msg));
            None
        }
    }
}

/// Parses an optional yen amount: empty is `Some(None)`
/// (unset), a valid amount `Some(Some(v))`, and bad input
/// pushes the message and returns `None`.
fn parse_optional_amount(
    raw: &str,
    field: &'static str,
    message: &'static str,
    errors: &mut Vec<FieldError>,
) -> Option<Option<i64>> {
    let text = raw.trim();
    if text.is_empty() {
        return Some(None);
    }
    if !text.chars().all(|c| c.is_ascii_digit()) {
        errors.push(FieldError::new(field, message));
        return None;
    }
    match text.parse::<i64>() {
        Ok(v) if (1..=MAX_PRICE_YEN).contains(&v) => Some(Some(v)),
        _ => {
            errors.push(FieldError::new(field, message));
            None
        }
    }
}

/// Parses an optional bounded integer the same way: empty is
/// `None` without an error (the caller applies its default).
fn parse_optional_int(
    raw: &str,
    field: &'static str,
    min: i64,
    max: i64,
    message: &'static str,
    errors: &mut Vec<FieldError>,
) -> Option<i64> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    match text.parse::<i64>() {
        Ok(v) if (min..=max).contains(&v) => Some(v),
        _ => {
            errors.push(FieldError::new(field, message));
            None
        }
    }
}

/// A trimmed optional text with a character ceiling: empty is
/// `None` (unset).
fn optional_text(
    raw: &str,
    max_chars: usize,
    field: &'static str,
    message: &'static str,
    errors: &mut Vec<FieldError>,
) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() > max_chars {
        errors.push(FieldError::new(field, message));
        return None;
    }
    Some(text.to_string())
}

/// A trimmed free text with a character ceiling (empty stays
/// empty — the field itself is optional).
fn required_text(
    raw: &str,
    max_chars: usize,
    field: &'static str,
    message: &'static str,
    errors: &mut Vec<FieldError>,
) -> String {
    let text = raw.trim().to_string();
    if text.chars().count() > max_chars {
        errors.push(FieldError::new(field, message));
    }
    text
}

/// Parses a `datetime-local` value as JST wall-clock time into
/// UTC. Empty is `Some(None)`; a malformed value pushes an
/// error and returns `None`.
fn parse_jst_local(
    raw: &str,
    field: &'static str,
    errors: &mut Vec<FieldError>,
) -> Option<Option<chrono::DateTime<chrono::Utc>>> {
    let text = raw.trim();
    if text.is_empty() {
        return Some(None);
    }
    match chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M") {
        Ok(naive) => {
            let utc = naive - chrono::Duration::hours(9);
            Some(Some(
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(utc, chrono::Utc),
            ))
        }
        Err(_) => {
            errors.push(FieldError::new(field, DATETIME_FORMAT_MESSAGE_BARE));
            None
        }
    }
}

/// Creates a product with its tags, its image list and the
/// `product.create` audit row, in one transaction.
///
/// `adjustment_note` tags the stock adjustments this save
/// writes (the CSV import names its batch); the product
/// form passes `None`.
pub async fn create_product(
    pool: &PgPool,
    admin_id: i64,
    valid: &ValidProduct,
    adjustment_note: Option<&str>,
) -> Result<i64, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = create_product_tx(&mut tx, admin_id, valid, adjustment_note).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(id)
}

/// The create above inside a caller-owned transaction: the
/// CSV batch apply runs every group through here so the whole
/// batch commits or rolls back as one unit.
pub async fn create_product_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    valid: &ValidProduct,
    adjustment_note: Option<&str>,
) -> Result<i64, AppError> {
    let fields = fields_of(valid, &valid.image_path);
    let id = admin_product_repository::create_product(tx, &fields).await?;
    tag_repository::replace_product_tags(tx, id, &valid.tag_ids).await?;
    save_matrix(tx, id, &valid.matrix, None, admin_id, adjustment_note).await?;
    product_image_repository::replace_images(tx, id, &valid.images).await?;
    audit_service::record(tx, admin_id, "product.create", "product", Some(id)).await?;
    Ok(id)
}

/// Updates a product's fields, tags and image list with
/// the `product.update` audit row, in one transaction.
///
/// `adjustment_note` tags the stock adjustments this save
/// writes, exactly as on [`create_product`].
///
/// `expected` is the staleness guard: the `updated_at`
/// (epoch microseconds) the edit form was opened with. When
/// present, the save refuses with [`AppError::Conflict`] if
/// another save has touched the product since — a stale form
/// must never silently overwrite newer prices or stock. The
/// CSV import passes `None`: its preview/revalidation flow
/// is its own guard.
pub async fn update_product(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    valid: &ValidProduct,
    adjustment_note: Option<&str>,
    expected: Option<i64>,
) -> Result<(), AppError> {
    let existing = existing_matrix(pool, id).await?;
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    update_product_tx(
        &mut tx,
        admin_id,
        id,
        valid,
        &existing,
        adjustment_note,
        expected,
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// The update above inside a caller-owned transaction: the
/// CSV batch apply runs every group through here so the whole
/// batch commits or rolls back as one unit. `existing` is
/// the variant matrix the save diffs against, read by the
/// caller exactly as the form save reads it.
pub async fn update_product_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    id: i64,
    valid: &ValidProduct,
    existing: &ExistingMatrix,
    adjustment_note: Option<&str>,
    expected: Option<i64>,
) -> Result<(), AppError> {
    let fields = fields_of(valid, &valid.image_path);
    if let Some(expected) = expected {
        let current = admin_product_repository::product_updated_at_for_update(tx, id).await?;
        if current.map(|t| t.timestamp_micros()) != Some(expected) {
            return Err(AppError::Conflict(
                "この商品は既に更新されています。最新の内容を確認してから保存してください。"
                    .to_string(),
            ));
        }
    }
    admin_product_repository::update_product(tx, id, &fields, admin_id).await?;
    tag_repository::replace_product_tags(tx, id, &valid.tag_ids).await?;
    save_matrix(
        tx,
        id,
        &valid.matrix,
        Some(existing),
        admin_id,
        adjustment_note,
    )
    .await?;
    product_image_repository::replace_images(tx, id, &valid.images).await?;
    audit_service::record(tx, admin_id, "product.update", "product", Some(id)).await?;
    Ok(())
}

/// Archives one product (`product.archive`). Products are
/// never hard-deleted from the admin side: order history refers
/// to them.
pub async fn archive_product(pool: &PgPool, admin_id: i64, id: i64) -> Result<(), AppError> {
    set_status(pool, admin_id, id, "archived", "product.archive").await
}

/// Unarchives one product back to draft (`product.unarchive`) —
/// never straight to public: re-publication is the operator's
/// separate, deliberate act on the edit form.
pub async fn unarchive_product(pool: &PgPool, admin_id: i64, id: i64) -> Result<(), AppError> {
    set_status(pool, admin_id, id, "draft", "product.unarchive").await
}

/// One status move with its audit row, in one transaction.
async fn set_status(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    status: &str,
    action: &str,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    set_status_tx(&mut tx, admin_id, id, status, action).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// The status move above inside a caller-owned transaction:
/// the CSV batch apply archives through here so the archive
/// commits or rolls back with the rest of the batch.
pub async fn set_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    id: i64,
    status: &str,
    action: &str,
) -> Result<(), AppError> {
    admin_product_repository::set_product_status(tx, id, status, admin_id).await?;
    audit_service::record(tx, admin_id, action, "product", Some(id)).await?;
    Ok(())
}

/// Bulk status change from the list's selection bar: every
/// selected product moves to `status`, each with its own
/// `product.status` audit row, all in one transaction.
/// Products already in the target state are skipped silently
/// (re-applying the current state is not an action). When the
/// target is `public`, products with no image are skipped
/// too — the publish gate — and counted separately, so the
/// flash reports them instead of silently half-succeeding.
/// Returns (changed, skipped-for-no-image).
pub async fn bulk_set_status(
    pool: &PgPool,
    admin_id: i64,
    ids: &[i64],
    status: &str,
) -> Result<(usize, usize), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The round-trip count is constant: one batched status read,
    // one batched update, one multi-row audit insert. Repeated IDs
    // in a crafted form are de-duplicated: one product changes
    // once and earns one audit row.
    let current = admin_product_repository::statuses_of(&mut tx, ids).await?;
    let mut seen = std::collections::HashSet::new();
    let mut changed_ids: Vec<i64> = Vec::new();
    let mut skipped_no_image = 0usize;
    for &id in ids {
        if !seen.insert(id) {
            continue;
        }
        match current.iter().find(|(pid, _, _)| *pid == id) {
            Some((_, s, has_image)) if s != status => {
                if status == "public" && !has_image {
                    skipped_no_image += 1;
                } else {
                    changed_ids.push(id);
                }
            }
            _ => {}
        }
    }
    if !changed_ids.is_empty() {
        admin_product_repository::set_product_status_many(&mut tx, &changed_ids, status, admin_id)
            .await?;
        audit_service::record_many(&mut tx, admin_id, "product.status", "product", &changed_ids)
            .await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok((changed_ids.len(), skipped_no_image))
}

/// The bulk bar's flash text, keyed by target status. Kept
/// beside the writes it names. A publish that skipped
/// imageless products says how many were held back by the
/// publish gate, so a partial success is never silent.
pub fn bulk_flash(status: &str, count: usize, skipped_no_image: usize) -> String {
    let base = match status {
        "public" => format!("{count}件の商品を公開しました"),
        "archived" => format!("{count}件の商品をアーカイブしました"),
        _ => format!("{count}件の商品を下書きにしました"),
    };
    if status == "public" && skipped_no_image > 0 {
        format!("{base}（画像のない{skipped_no_image}件は公開できませんでした）")
    } else {
        base
    }
}

/// Groups field errors by field name for view construction.
pub fn errors_by_field(errors: &[FieldError]) -> BTreeMap<&'static str, String> {
    errors
        .iter()
        .map(|e| (e.field, e.message.clone()))
        .collect()
}

/// Builds the repository field set from the validated form.
/// The image path is formatted by the caller into a binding it
/// owns, because the field set borrows it.
fn fields_of<'a>(
    valid: &'a ValidProduct,
    image_path: &'a str,
) -> admin_product_repository::ProductFields<'a> {
    admin_product_repository::ProductFields {
        name: &valid.name,
        description: &valid.description,
        price_yen: valid.price_yen,
        category: &valid.category,
        category_id: valid.category_id,
        image_path,
        is_event: valid.is_event,
        status: &valid.status,
        product_code: valid.product_code.as_deref(),
        brand: &valid.brand,
        manufacturer: &valid.manufacturer,
        country_of_origin: &valid.country_of_origin,
        spec_text: &valid.spec_text,
        features_text: &valid.features_text,
        description_sp: &valid.description_sp,
        search_keywords: &valid.search_keywords,
        tax_class_id: valid.tax_class_id,
        cost_yen: valid.cost_yen,
        sale_price_yen: valid.sale_price_yen,
        sale_starts_at: valid.sale_starts_at,
        sale_ends_at: valid.sale_ends_at,
        is_new: valid.is_new,
        discount_percent: valid.discount_percent,
        is_preorder: valid.is_preorder,
        is_staff_pick: valid.is_staff_pick,
        display_order: valid.display_order,
        slug: valid.slug.as_deref(),
        seo_title: &valid.seo_title,
        meta_description: &valid.meta_description,
        track_stock: valid.track_stock,
        allow_backorder: valid.allow_backorder,
        low_stock_threshold: valid.low_stock_threshold,
        points_earn_percent: valid.points_earn_percent,
        weight_grams: valid.weight_grams,
        size_length_mm: valid.size_length_mm,
        size_width_mm: valid.size_width_mm,
        size_height_mm: valid.size_height_mm,
        cool_delivery: valid.cool_delivery,
        mail_delivery: valid.mail_delivery,
        free_shipping: valid.free_shipping,
        undeliverable_regions: &valid.undeliverable_regions,
        lead_time_days: valid.lead_time_days,
        publish_starts_at: valid.publish_starts_at,
        publish_ends_at: valid.publish_ends_at,
    }
}

/// Loads a product's stored variation matrix: its axes with
/// options and its combinations with their order protection.
/// The edit form renders from this and the matrix validation
/// checks removals against it.
pub async fn existing_matrix(pool: &PgPool, product_id: i64) -> Result<ExistingMatrix, AppError> {
    let axes = variant_repository::axes_with_options(pool, product_id).await?;
    let variants = variant_repository::list_variants(pool, product_id).await?;
    let links = variant_repository::links_for(pool, product_id).await?;
    let ordered = variant_repository::ordered_variant_ids(pool, product_id).await?;
    let variants = variants
        .into_iter()
        .map(|v| {
            let mut option_ids: Vec<i64> = links
                .iter()
                .filter(|(variant_id, _)| *variant_id == v.id)
                .map(|(_, option_id)| *option_id)
                .collect();
            option_ids.sort_unstable();
            ExistingVariant {
                ordered: ordered.contains(&v.id),
                id: v.id,
                stock: v.stock,
                option_ids,
            }
        })
        .collect();
    Ok(ExistingMatrix { axes, variants })
}

/// One combination's full grid state for the edit form: its
/// option values plus every per-combination field.
pub struct ComboFull {
    /// Option values in axis order.
    pub values: Vec<String>,
    /// Stock on hand.
    pub stock: i32,
    /// Price override; `None` = product price.
    pub price_yen: Option<i64>,
    /// SKU; empty when unset.
    pub sku: String,
    /// JAN code; empty when unset.
    pub jan_code: String,
    /// Image path; empty when unset.
    pub image_path: String,
    /// Whether the combination is enabled.
    pub is_enabled: bool,
    /// Expected incoming quantity.
    pub incoming_qty: i32,
    /// Expected incoming date (`YYYY-MM-DD`); empty when unset.
    pub incoming_date: String,
}

/// Returns a product's combinations with all grid fields, for
/// the edit form's prefill: the value rows of
/// [`combos_with_values`] zipped with the stored variants (both
/// read in variant-ID order, so they line up row for row).
pub async fn combos_full(pool: &PgPool, product_id: i64) -> Result<Vec<ComboFull>, AppError> {
    let values = variant_repository::combos_with_values(pool, product_id).await?;
    let variants = variant_repository::list_variants(pool, product_id).await?;
    Ok(values
        .into_iter()
        .zip(variants.iter())
        .map(|((values, _stock), v)| ComboFull {
            values,
            stock: v.stock,
            price_yen: v.price_yen,
            sku: v.sku.clone().unwrap_or_default(),
            jan_code: v.jan_code.clone().unwrap_or_default(),
            image_path: v.image_path.clone().unwrap_or_default(),
            is_enabled: v.is_enabled,
            incoming_qty: v.incoming_qty,
            incoming_date: v
                .incoming_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
        })
        .collect())
}

/// Returns a product's attached images with alt texts, for the
/// edit form's gallery prefill.
pub async fn product_images_detailed(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<(String, String)>, AppError> {
    product_image_repository::list_detailed(pool, product_id).await
}

/// Returns a product's combinations as (option values in axis
/// order, stock) pairs, for the edit form's grid prefill.
pub async fn combos_with_values(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<(Vec<String>, i32)>, AppError> {
    Ok(variant_repository::combos_with_values(pool, product_id).await?)
}

/// Validates the variation part of the form: axis and option
/// limits, non-empty duplicate-free texts, the combination
/// ceiling, one valid stock per combination, and — on edit —
/// the removal protection. Errors attach to the `variants`
/// field, which the form renders under the section.
fn validate_matrix(
    input: &ProductFormInput,
    existing: Option<&ExistingMatrix>,
    taken: &admin_product_repository::TakenIdentifiers,
    image_candidates: &[String],
    errors: &mut Vec<FieldError>,
) -> Option<ValidMatrix> {
    let mut bad = false;
    let mut push = |errors: &mut Vec<FieldError>, message: String| {
        errors.push(FieldError::new("variants", &message));
        bad = true;
    };
    if input.axes.len() > MAX_AXES {
        push(errors, format!("バリエーションは{MAX_AXES}つまでです"));
    }
    let mut axes: Vec<ValidAxis> = Vec::new();
    let mut seen_names: Vec<String> = Vec::new();
    for axis_in in &input.axes {
        let name = axis_in.name.trim().to_string();
        if name.is_empty() {
            push(errors, "バリエーション名を入力してください".to_string());
        } else if seen_names.contains(&name) {
            push(
                errors,
                format!("バリエーション名「{name}」が重複しています"),
            );
        } else {
            seen_names.push(name.clone());
        }
        if axis_in.options.is_empty() || axis_in.options.len() > MAX_OPTIONS {
            push(
                errors,
                format!("「{name}」の選択肢は1〜{MAX_OPTIONS}つにしてください"),
            );
        }
        let mut seen_values: Vec<String> = Vec::new();
        let mut options: Vec<ValidOption> = Vec::new();
        for option_in in &axis_in.options {
            let value = option_in.value.trim().to_string();
            if value.is_empty() {
                push(errors, format!("「{name}」の選択肢を入力してください"));
            } else if seen_values.contains(&value) {
                push(
                    errors,
                    format!("「{name}」の選択肢『{value}』が重複しています"),
                );
            } else {
                seen_values.push(value.clone());
            }
            let id = match parse_stored_id(&option_in.id) {
                Some(id) => id,
                None => {
                    push(errors, VARIANT_SPEC_INVALID_MESSAGE.to_string());
                    None
                }
            };
            options.push(ValidOption { id, value });
        }
        let id = match parse_stored_id(&axis_in.id) {
            Some(id) => id,
            None => {
                push(errors, VARIANT_SPEC_INVALID_MESSAGE.to_string());
                None
            }
        };
        axes.push(ValidAxis { id, name, options });
    }

    // The combination count is the axes' cartesian size (one for
    // an axis-less product: its default variant).
    let combos: usize = if axes.is_empty() {
        1
    } else {
        axes.iter().map(|a| a.options.len()).product()
    };
    if combos > MAX_COMBOS {
        push(
            errors,
            format!("組み合わせは{MAX_COMBOS}通りまでです（現在{combos}通り）"),
        );
    }
    let mut valid_combos: Vec<ValidCombo> = Vec::new();
    if input.combos.len() != combos {
        push(errors, "在庫の入力が組み合わせと一致しません".to_string());
    } else {
        let mut seen_skus: Vec<String> = Vec::new();
        let mut seen_jans: Vec<String> = Vec::new();
        for combo in &input.combos {
            let stock = parse_amount(
                &combo.stock,
                &AmountRule {
                    field: "variants",
                    empty_msg: "在庫数を入力してください",
                    integer_msg: "在庫数は0以上の整数で入力してください",
                    max_msg: "在庫数は999,999以下で入力してください",
                    min: 0,
                    max: MAX_STOCK,
                },
                errors,
            );
            let price_yen = parse_optional_amount(
                &combo.price_yen,
                "variants",
                "組み合わせ価格は1円以上の整数で入力してください",
                errors,
            )
            .flatten();
            let sku = optional_text(
                &combo.sku,
                64,
                "variants",
                "SKUは64文字以内で入力してください",
                errors,
            );
            if let Some(s) = &sku {
                if seen_skus.iter().any(|x| x == s) {
                    push(errors, format!("SKU「{s}」が重複しています"));
                } else if taken.skus.iter().any(|x| x == s) {
                    push(errors, format!("SKU「{s}」は既に使用されています"));
                }
                seen_skus.push(s.clone());
            }
            let jan_code = optional_text(
                &combo.jan_code,
                64,
                "variants",
                "JANコードは64文字以内で入力してください",
                errors,
            );
            if let Some(j) = &jan_code {
                if !j.chars().all(|c| c.is_ascii_digit()) {
                    push(errors, "JANコードは半角数字で入力してください".to_string());
                }
                if seen_jans.iter().any(|x| x == j) {
                    push(errors, format!("JANコード「{j}」が重複しています"));
                } else if taken.jan_codes.iter().any(|x| x == j) {
                    push(errors, format!("JANコード「{j}」は既に使用されています"));
                }
                seen_jans.push(j.clone());
            }
            let image_path = {
                let path = combo.image_path.trim();
                if path.is_empty() {
                    None
                } else if image_candidates.iter().any(|c| c == path) {
                    Some(path.to_string())
                } else {
                    push(
                        errors,
                        "組み合わせ画像は商品の画像から選択してください".to_string(),
                    );
                    None
                }
            };
            let incoming_qty = parse_optional_int(
                &combo.incoming_qty,
                "variants",
                0,
                MAX_STOCK,
                "入荷予定数は0以上の整数で入力してください",
                errors,
            )
            .unwrap_or(0) as i32;
            let incoming_date = {
                let text = combo.incoming_date.trim();
                if text.is_empty() {
                    None
                } else {
                    match NaiveDate::parse_from_str(text, "%Y-%m-%d") {
                        Ok(d) => Some(d),
                        Err(_) => {
                            push(errors, "入荷予定日の形式が正しくありません".to_string());
                            None
                        }
                    }
                }
            };
            match stock {
                Some(v) => valid_combos.push(ValidCombo {
                    stock: v as i32,
                    price_yen,
                    sku,
                    jan_code,
                    image_path,
                    is_enabled: combo.is_enabled,
                    incoming_qty,
                    incoming_date,
                }),
                None => {
                    bad = true;
                    break;
                }
            }
        }
    }
    if bad {
        return None;
    }
    if let Some(existing) = existing
        && let Some(message) = matrix_removal_block(&axes, existing)
    {
        errors.push(FieldError::new("variants", &message));
        return None;
    }
    Some(ValidMatrix {
        axes,
        combos: valid_combos,
    })
}

/// Parses a stored axis/option ID as the form posts it: empty
/// means a new row (`Some(None)`); a non-numeric value is a
/// crafted post and fails (`None`).
fn parse_stored_id(raw: &str) -> Option<Option<i64>> {
    let text = raw.trim();
    if text.is_empty() {
        return Some(None);
    }
    text.parse::<i64>().ok().map(Some)
}

/// The combination sets a form describes, over stored option
/// IDs only: a combination containing a new option cannot be an
/// existing variant, so it takes no part in survival matching.
/// Each set is sorted for comparison.
fn desired_id_sets(axes: &[ValidAxis]) -> Vec<Vec<i64>> {
    let per_axis: Vec<Vec<Option<i64>>> = axes
        .iter()
        .map(|a| a.options.iter().map(|o| o.id).collect())
        .collect();
    cartesian(&per_axis)
        .into_iter()
        .filter(|combo| combo.iter().all(Option::is_some))
        .map(|combo| {
            let mut set: Vec<i64> = combo
                .into_iter()
                .map(|id| id.expect("the filter keeps only complete combinations"))
                .collect();
            set.sort_unstable();
            set
        })
        .collect()
}

/// Checks a matrix edit against the removal protection: an
/// existing combination survives when nothing it uses was
/// removed and its combination is still offered. Returns the
/// error message for the first ordered combination that would
/// not survive — naming the removed option responsible when
/// there is one — or `None` when the edit is allowed.
fn matrix_removal_block(axes: &[ValidAxis], existing: &ExistingMatrix) -> Option<String> {
    let surviving: Vec<i64> = axes
        .iter()
        .flat_map(|a| a.options.iter().filter_map(|o| o.id))
        .collect();
    let desired = desired_id_sets(axes);
    let mut claimed = vec![false; desired.len()];
    for variant in &existing.variants {
        // The default variant (no options) is exempt: it is the
        // pre-variant state, not a combination a customer picked,
        // and converting a plain product into a variant product
        // (the spec section 10 fixture path) replaces it. Its
        // order lines keep their snapshot label; the foreign key
        // sets their variant_id to NULL.
        if variant.option_ids.is_empty() {
            continue;
        }
        let removed: Vec<i64> = variant
            .option_ids
            .iter()
            .copied()
            .filter(|id| !surviving.contains(id))
            .collect();
        let mut survives = removed.is_empty();
        if survives {
            match desired.iter().position(|set| *set == variant.option_ids) {
                Some(i) if !claimed[i] => claimed[i] = true,
                _ => survives = false,
            }
        }
        if !survives && variant.ordered {
            // Name the first removed option, in axis order: it
            // is the thing the operator must keep (or zero out
            // through stock) instead.
            for axis in &existing.axes {
                for option in &axis.options {
                    if removed.contains(&option.id) {
                        return Some(format!(
                            "{}『{}』は注文があるため削除できません",
                            axis.axis.name, option.value
                        ));
                    }
                }
            }
            return Some(
                "注文のあるバリエーションが含まれるため、構成を変更できません".to_string(),
            );
        }
    }
    None
}

/// Writes a validated matrix inside the caller's transaction:
/// removed axes and options are deleted, kept rows are upserted
/// by ID (so renames and reorders preserve combination identity),
/// combinations are regenerated, survivors keep their rows with
/// the grid's stock, and new combinations are inserted.
/// Validation has already proven no ordered combination is lost.
async fn save_matrix(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    product_id: i64,
    matrix: &ValidMatrix,
    existing: Option<&ExistingMatrix>,
    admin_id: i64,
    adjustment_note: Option<&str>,
) -> Result<(), AppError> {
    let empty = ExistingMatrix {
        axes: Vec::new(),
        variants: Vec::new(),
    };
    let existing = existing.unwrap_or(&empty);

    let form_axis_ids: Vec<i64> = matrix.axes.iter().filter_map(|a| a.id).collect();
    let form_option_ids: Vec<i64> = matrix
        .axes
        .iter()
        .flat_map(|a| a.options.iter().filter_map(|o| o.id))
        .collect();
    for axis in &existing.axes {
        if !form_axis_ids.contains(&axis.axis.id) {
            variant_repository::delete_axis(tx, axis.axis.id).await?;
        } else {
            for option in &axis.options {
                if !form_option_ids.contains(&option.id) {
                    variant_repository::delete_option(tx, option.id).await?;
                }
            }
        }
    }

    // Upsert axes and options in form order; the final option IDs
    // per axis are the combination space.
    let mut final_axes: Vec<Vec<i64>> = Vec::new();
    for (axis_idx, axis) in matrix.axes.iter().enumerate() {
        let axis_id = match axis.id {
            Some(id) => {
                variant_repository::update_axis(tx, id, &axis.name, axis_idx as i32).await?;
                id
            }
            None => {
                variant_repository::insert_axis(tx, product_id, &axis.name, axis_idx as i32).await?
            }
        };
        let mut option_ids = Vec::new();
        for (option_idx, option) in axis.options.iter().enumerate() {
            let option_id = match option.id {
                Some(id) => {
                    variant_repository::update_option(tx, id, &option.value, option_idx as i32)
                        .await?;
                    id
                }
                None => {
                    variant_repository::insert_option(tx, axis_id, &option.value, option_idx as i32)
                        .await?
                }
            };
            option_ids.push(option_id);
        }
        final_axes.push(option_ids);
    }

    let combos = cartesian(&final_axes);
    let sets: Vec<Vec<i64>> = combos
        .iter()
        .map(|combo| {
            let mut set = combo.clone();
            set.sort_unstable();
            set
        })
        .collect();

    // A stored combination survives when every option it uses is
    // still offered (its effective set is itself) and its set is
    // one of the regenerated combinations; the first claimant in
    // ID order wins a set two stored rows collapse onto.
    let mut claimed: Vec<Option<usize>> = vec![None; existing.variants.len()];
    let mut combo_taken = vec![false; combos.len()];
    for (vi, variant) in existing.variants.iter().enumerate() {
        let effective: Vec<i64> = variant
            .option_ids
            .iter()
            .copied()
            .filter(|id| form_option_ids.contains(id))
            .collect();
        if effective.len() != variant.option_ids.len() {
            continue;
        }
        if let Some(ci) = sets.iter().position(|set| *set == effective)
            && !combo_taken[ci]
        {
            combo_taken[ci] = true;
            claimed[vi] = Some(ci);
        }
    }

    // Re-sign survivors in two passes: a stored signature could
    // equal another survivor's new one mid-flight (an axis
    // reorder swaps them), so every survivor first takes a
    // temporary signature no real combination can have. Each
    // pass is ONE batched update; the first pass also
    // parks SKU and JAN at NULL, because their unique indexes
    // would collide mid-flight on a swap just like signatures.
    let survivors: Vec<(usize, usize)> = existing
        .variants
        .iter()
        .enumerate()
        .filter_map(|(vi, _)| claimed[vi].map(|ci| (vi, ci)))
        .collect();
    if !survivors.is_empty() {
        let temp_signatures: Vec<String> = survivors
            .iter()
            .map(|(vi, _)| format!("__moving__{}", existing.variants[*vi].id))
            .collect();
        let parked: Vec<variant_repository::VariantFields> = survivors
            .iter()
            .map(|(_, ci)| {
                let mut fields = matrix.combos[*ci].fields();
                fields.sku = None;
                fields.jan_code = None;
                fields
            })
            .collect();
        let updates: Vec<variant_repository::VariantUpdate<'_>> = survivors
            .iter()
            .zip(&temp_signatures)
            .zip(&parked)
            .map(
                |(((vi, _), signature), fields)| variant_repository::VariantUpdate {
                    id: existing.variants[*vi].id,
                    signature,
                    fields,
                },
            )
            .collect();
        variant_repository::update_variants(tx, &updates).await?;
    }
    // The stock ledger is append-only: a
    // combination with ledger history cannot be deleted
    // (its FK is RESTRICT), so refuse the save up front
    // with a field error instead of failing mid-transaction
    // on the constraint.
    let removed_ids: Vec<i64> = existing
        .variants
        .iter()
        .enumerate()
        .filter(|(vi, _)| claimed[*vi].is_none())
        .map(|(_, variant)| variant.id)
        .collect();
    if !removed_ids.is_empty()
        && variant_repository::variants_with_stock_history(tx, &removed_ids).await?
    {
        return Err(AppError::Validation(vec![FieldError::new(
            "variants",
            "在庫の履歴があるバリエーションは削除できません。販売を停止して残してください",
        )]));
    }
    // Combinations that did not survive are removed before the
    // final pass, so a survivor can take a removed row's
    // signature or SKU without meeting it mid-flight.
    for (vi, variant) in existing.variants.iter().enumerate() {
        if claimed[vi].is_none() {
            variant_repository::delete_variant(tx, variant.id).await?;
        }
    }
    if !survivors.is_empty() {
        let final_signatures: Vec<String> = survivors
            .iter()
            .map(|(_, ci)| signature_for(&combos[*ci]))
            .collect();
        let full: Vec<variant_repository::VariantFields> = survivors
            .iter()
            .map(|(_, ci)| matrix.combos[*ci].fields())
            .collect();
        let updates: Vec<variant_repository::VariantUpdate<'_>> = survivors
            .iter()
            .zip(&final_signatures)
            .zip(&full)
            .map(
                |(((vi, _), signature), fields)| variant_repository::VariantUpdate {
                    id: existing.variants[*vi].id,
                    signature,
                    fields,
                },
            )
            .collect();
        variant_repository::update_variants(tx, &updates).await?;
    }
    // Combinations that are new are inserted next; their IDs
    // join the link and ledger batches below.
    let mut inserted: Vec<(usize, i64)> = Vec::new();
    for (ci, combo) in combos.iter().enumerate() {
        if !combo_taken[ci] {
            let variant_id = variant_repository::insert_variant(
                tx,
                product_id,
                &signature_for(combo),
                &matrix.combos[ci].fields(),
            )
            .await?;
            inserted.push((ci, variant_id));
        }
    }
    // Every touched combination's links are rewritten with one
    // DELETE + one multi-row INSERT, and every stock movement
    // lands in the ledger as one insert: a manual change
    // through the grid for survivors, the opening stock for new
    // combinations. Movements are collected in the same order
    // the rows were written before the batching.
    let mut link_variants: Vec<i64> = Vec::new();
    let mut link_pairs: Vec<(i64, i64)> = Vec::new();
    let mut adjustments: Vec<crate::repositories::stock_ledger_repository::NewAdjustment<'_>> =
        Vec::new();
    for (vi, ci) in &survivors {
        let variant = &existing.variants[*vi];
        link_variants.push(variant.id);
        link_pairs.extend(combos[*ci].iter().map(|option_id| (variant.id, *option_id)));
        adjustments.push(
            crate::repositories::stock_ledger_repository::NewAdjustment {
                variant_id: variant.id,
                delta: matrix.combos[*ci].stock - variant.stock,
                reason: "manual",
                order_id: None,
                admin_id: Some(admin_id),
                note: adjustment_note,
            },
        );
    }
    for (ci, variant_id) in &inserted {
        link_variants.push(*variant_id);
        link_pairs.extend(
            combos[*ci]
                .iter()
                .map(|option_id| (*variant_id, *option_id)),
        );
        adjustments.push(
            crate::repositories::stock_ledger_repository::NewAdjustment {
                variant_id: *variant_id,
                delta: matrix.combos[*ci].stock,
                reason: "initial",
                order_id: None,
                admin_id: Some(admin_id),
                note: adjustment_note,
            },
        );
    }
    variant_repository::replace_links(tx, &link_variants, &link_pairs).await?;
    crate::repositories::stock_ledger_repository::insert_adjustments(tx, &adjustments).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_raster_type_from_its_magic_bytes() {
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        assert_eq!(uploaded_image_extension(&png), Some("png"));
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00];
        assert_eq!(uploaded_image_extension(&jpeg), Some("jpg"));
        assert_eq!(uploaded_image_extension(b"GIF89a...."), Some("gif"));
        assert_eq!(uploaded_image_extension(b"GIF87a...."), Some("gif"));
        assert_eq!(
            uploaded_image_extension(b"RIFF\x24\x00\x00\x00WEBPVP8 "),
            Some("webp")
        );
    }

    #[test]
    fn refuses_svg_and_other_non_raster_bytes() {
        assert_eq!(
            uploaded_image_extension(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"),
            None
        );
        assert_eq!(uploaded_image_extension(b"plain text"), None);
        assert_eq!(uploaded_image_extension(b""), None);
        assert_eq!(
            uploaded_image_extension(b"RIFF\x24\x00\x00\x00WAVEfmt "),
            None
        );
    }

    #[test]
    fn upload_paths_must_stay_inside_the_uploads_store() {
        // A built-in asset is not an upload: only a product
        // that already carries it may keep it (via `attached`).
        assert!(!is_upload_path("/static/images/product-rust-tshirt.svg"));
        assert!(!is_upload_path("/static/images/uploads/../secret.png"));
        assert!(!is_upload_path("https://example.com/x.png"));
        assert!(!is_upload_path("/static/images/uploads/no-such-file.png"));
    }

    #[test]
    fn stored_name_prefixes_the_first_five_letters() {
        let name = stored_image_filename(Some("summer-shirt-front.png"), "png");
        assert!(name.starts_with("summe-"), "{name}");
        assert!(name.ends_with(".png"), "{name}");
    }

    #[test]
    fn stored_name_uses_the_detected_extension_not_the_clients() {
        let name = stored_image_filename(Some("photo.jpeg"), "png");
        assert!(name.starts_with("photo-"), "{name}");
        assert!(name.ends_with(".png"), "{name}");
        assert!(!name.contains(".jpeg"), "{name}");
    }

    #[test]
    fn stored_name_sanitises_a_traversal_filename() {
        let name = stored_image_filename(Some("../../evil.png"), "png");
        assert!(name.starts_with("evil-"), "{name}");
        assert!(!name.contains('/'), "{name}");
        assert!(!name.contains('\\'), "{name}");
    }

    #[test]
    fn stored_name_keeps_japanese_letters() {
        let name = stored_image_filename(Some("夏用シャツ正面.jpg"), "jpg");
        assert!(name.starts_with("夏用シャツ-"), "{name}");
    }

    #[test]
    fn stored_name_falls_back_to_the_bare_uuid() {
        for original in [None, Some("★★★.png"), Some("")] {
            let name = stored_image_filename(original, "png");
            let stem = name.strip_suffix(".png").expect("extension");
            assert!(Uuid::parse_str(stem).is_ok(), "{name}");
        }
    }
}
