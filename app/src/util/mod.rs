//! Leaf utilities shared across the crate.
//!
//! Small, single-purpose helpers with no business logic of
//! their own: the static-asset fingerprint, CSV transfer,
//! HMAC helpers, HTML sanitising, upload image paths,
//! script-safe JSON, LIKE-pattern escaping, log plumbing,
//! Markdown rendering, pagination, the rate-limit knobs,
//! and the blog embed shortcodes.
//! The crate-wide vocabulary ([`crate::error`], [`crate::pricing`],
//! [`crate::session`], [`crate::messages`]) deliberately stays
//! at the crate root instead of moving here.

pub mod asset_fingerprint;
pub mod csv_transfer;
pub mod hmac_util;
pub mod html_sanitize;
pub mod image_paths;
pub mod json_for_script;
pub mod like_pattern;
pub mod logging;
pub mod markdown;
pub mod pagination;
pub mod rate_limit_knobs;
pub mod shortcode;
pub mod xml;
