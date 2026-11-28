//! Storefront paths for uploaded product image derivatives.
//!
//! An uploaded original lives at
//! `/static/images/uploads/<name>.<ext>`; beside it the
//! upload hook writes two resized copies:
//! `<name>_thumb.<dext>` for cards and the detail gallery
//! strip, and `<name>_medium.<dext>` for the detail page's
//! main image (the original remains the lightbox view). The
//! derivative extension `<dext>` follows the original's
//! family: a JPEG original's derivatives stay JPEG (lossy,
//! at the configured quality — a lossless copy of a photo
//! stays needlessly large), while PNG and WebP originals
//! get lossless WebP derivatives so transparency survives.
//! This module is the one place that knows the naming
//! rule — the generator writes through it and the model
//! methods the templates call read through it, so the two
//! sides cannot drift apart.

/// File-name suffix of the card-sized derivative, appended
/// to the original's stem.
pub const THUMB_SUFFIX: &str = "_thumb";

/// File-name suffix of the detail-sized derivative.
pub const MEDIUM_SUFFIX: &str = "_medium";

/// URL prefix of the uploads directory: derivatives exist
/// only for images under it. Anything else (seed art, hero
/// images, external URLs) is served as it is.
const UPLOADS_PREFIX: &str = "/static/images/uploads/";

/// The placeholder served when a product has no image at
/// all. Templates must never emit an empty `src`: an empty attribute resolves to the page
/// itself and renders as a broken-image glyph with the
/// alt text spilled into the layout — and the app.js
/// error fallback cannot catch it, because it keys on
/// `/static/` paths.
pub const NO_IMAGE_PATH: &str = "/static/images/no-image.svg";

/// The derivative path for an uploaded image, or the path
/// itself when no derivative applies: non-upload paths,
/// GIFs (a still derivative would freeze the animation) and
/// names without a raster extension keep the original on
/// every surface. The browser still falls back to the
/// original when a derivative file is missing, so returning
/// a derivative path for an upload whose generation failed
/// is safe.
fn derivative_path(path: &str, suffix: &str) -> String {
    if path.is_empty() {
        return NO_IMAGE_PATH.to_string();
    }
    if !path.starts_with(UPLOADS_PREFIX) {
        return path.to_string();
    }
    let Some((stem, ext)) = path.rsplit_once('.') else {
        return path.to_string();
    };
    let ext = ext.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp") {
        return path.to_string();
    }
    format!("{stem}{suffix}.{}", derivative_ext_for(&ext))
}

/// The extension a derivative of the given source extension
/// carries: JPEG sources stay JPEG, the other raster
/// sources become WebP. The generator decides from the
/// decoded format and lands on the same rule, because the
/// stored name's extension always matches the bytes (the
/// upload endpoint names files by their magic bytes).
pub fn derivative_ext_for(source_ext: &str) -> &'static str {
    match source_ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "jpg",
        _ => "webp",
    }
}

/// The card-sized derivative path for a product image path.
pub fn thumb_path(path: &str) -> String {
    derivative_path(path, THUMB_SUFFIX)
}

/// The detail-sized derivative path for a product image path.
pub fn medium_path(path: &str) -> String {
    derivative_path(path, MEDIUM_SUFFIX)
}

#[cfg(test)]
mod tests {
    use super::{medium_path, thumb_path};

    #[test]
    fn upload_paths_map_to_derivatives() {
        // A JPEG original's derivatives stay JPEG.
        let original = "/static/images/uploads/IMG_3-abc123.jpg";
        assert_eq!(
            thumb_path(original),
            "/static/images/uploads/IMG_3-abc123_thumb.jpg"
        );
        assert_eq!(
            medium_path(original),
            "/static/images/uploads/IMG_3-abc123_medium.jpg"
        );
        // PNG and WebP originals derive to WebP.
        assert_eq!(
            thumb_path("/static/images/uploads/a-b.png"),
            "/static/images/uploads/a-b_thumb.webp"
        );
        assert_eq!(
            thumb_path("/static/images/uploads/a-b.webp"),
            "/static/images/uploads/a-b_thumb.webp"
        );
        // Extension matching is case-insensitive.
        assert_eq!(
            thumb_path("/static/images/uploads/a-b.PNG"),
            "/static/images/uploads/a-b_thumb.webp"
        );
    }

    #[test]
    fn non_derivable_paths_pass_through() {
        // GIFs keep their animation on every surface.
        assert_eq!(
            thumb_path("/static/images/uploads/anim.gif"),
            "/static/images/uploads/anim.gif"
        );
        // Images outside the uploads directory are untouched.
        assert_eq!(
            thumb_path("/static/images/hero.jpg"),
            "/static/images/hero.jpg"
        );
        // A name with no extension is untouched.
        assert_eq!(
            medium_path("/static/images/uploads/noext"),
            "/static/images/uploads/noext"
        );
    }
}
