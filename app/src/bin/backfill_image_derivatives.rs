//! One-off backfill: generates the WebP
//! derivatives for product images uploaded before
//! derivatives existed (the upload hook covers new ones).
//! Safe to re-run — an image whose derivatives are already
//! present is skipped.
//!
//! Usage: `cargo run --bin backfill_image_derivatives -- [uploads_dir]`
//! The directory defaults to `resources/static/images/uploads`
//! and the sizes to the config defaults (480 / 1024).

use std::path::PathBuf;

/// The derivative spec, matching the `config.toml`
/// defaults (`image_thumb_max_px`, `image_medium_max_px`,
/// `image_jpeg_quality`).
const SPEC: app::services::admin::product_service::DerivativeSpec =
    app::services::admin::product_service::DerivativeSpec {
        thumb_max_px: 480,
        medium_max_px: 1024,
        jpeg_quality: 82,
    };

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/static/images/uploads")
        });
    let mut made = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let entries = std::fs::read_dir(&dir).expect("read the uploads directory");
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((stem, ext)) = name.rsplit_once('.') else {
            continue;
        };
        if stem.contains(app::util::image_paths::THUMB_SUFFIX)
            || stem.contains(app::util::image_paths::MEDIUM_SUFFIX)
        {
            continue;
        }
        if !matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp"
        ) {
            skipped += 1;
            continue;
        }
        let dext = app::util::image_paths::derivative_ext_for(ext);
        let thumb = dir.join(format!(
            "{stem}{}.{dext}",
            app::util::image_paths::THUMB_SUFFIX
        ));
        let medium = dir.join(format!(
            "{stem}{}.{dext}",
            app::util::image_paths::MEDIUM_SUFFIX
        ));
        if thumb.exists() && medium.exists() {
            skipped += 1;
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("read {name}: {e}");
                failed += 1;
                continue;
            }
        };
        match app::services::admin::product_service::generate_derivatives(&bytes, &dir, stem, &SPEC)
        {
            Ok(()) => made += 1,
            Err(e) => {
                eprintln!("{name}: {e}");
                failed += 1;
            }
        }
    }
    println!("derivatives generated for {made} images; skipped {skipped}; failed {failed}");
}
