//! Asset fingerprinting: the cache-busting `?v=` value
//! appended to stylesheet and script URLs in templates.

/// Returns the cache-busting fingerprint appended to asset URLs
/// (`?v=...`) emitted by the templates.
///
/// Page HTML is uncached but assets are cached for a day, so without
/// this an open tab could pair fresh markup with a stale script. The
/// value is a hash of the stylesheet and script files — the
/// only assets templates fingerprint — computed once per
/// process: it changes exactly when one of those files changes,
/// so no human has to bump a number, and an image upload never
/// busts the stylesheet cache. If the assets cannot be read, the
/// process start time stands in — a cache-buster must never
/// stop startup.
pub fn asset_fingerprint() -> &'static str {
    static FINGERPRINT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FINGERPRINT.get_or_init(|| fingerprint_of(&crate::routes::static_dir()))
}

/// Hashes the asset tree under `root`; on read failure, warns once
/// and falls back to the process start time.
fn fingerprint_of(root: &std::path::Path) -> String {
    match try_fingerprint_of(root) {
        Some(fingerprint) => fingerprint,
        None => {
            tracing::warn!(
                "could not read the static assets under {}; \
                 asset URLs fall back to a start-time fingerprint",
                root.display()
            );
            start_time_fingerprint()
        }
    }
}

/// Returns the fingerprint of the tree under `root`.
///
/// Every stylesheet or script file, in sorted relative-path
/// order, contributes its path and bytes to a fixed-key hash,
/// rendered as 16 lowercase hex characters; identical trees
/// hash identically. `None` on any read failure.
fn try_fingerprint_of(root: &std::path::Path) -> Option<String> {
    use std::hash::{Hash, Hasher};

    let mut files = Vec::new();
    collect_asset_files(root, root, &mut files).ok()?;
    files.sort();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (relative, path) in &files {
        relative.hash(&mut hasher);
        std::fs::read(path).ok()?.hash(&mut hasher);
    }
    Some(format!("{:016x}", hasher.finish()))
}

/// Collects every stylesheet or script file under `dir` as
/// (relative, full) path pairs.
///
/// Relative paths use `/` so the ordering — and the hash — is the
/// same on every platform.
fn collect_asset_files(
    dir: &std::path::Path,
    root: &std::path::Path,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_asset_files(&path, root, out)?;
        } else if path.is_file()
            && matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("css") | Some("js")
            )
        {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((relative, path));
        }
    }
    Ok(())
}

/// The fallback fingerprint: process start time in Unix seconds,
/// captured once so every caller agrees on the value.
fn start_time_fingerprint() -> String {
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let start = START.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    });
    start.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns a fresh, empty fixture directory unique to this test.
    fn fixture_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agentic-asset-fingerprint-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture dir");
        dir
    }

    #[test]
    fn fingerprint_is_stable_for_identical_contents() {
        let first = fixture_dir("stable-a");
        std::fs::write(first.join("style.css"), "body {}").expect("write");
        std::fs::create_dir_all(first.join("js")).expect("mkdir");
        std::fs::write(first.join("js/app.js"), "start()").expect("write");

        let second = fixture_dir("stable-b");
        std::fs::write(second.join("style.css"), "body {}").expect("write");
        std::fs::create_dir_all(second.join("js")).expect("mkdir");
        std::fs::write(second.join("js/app.js"), "start()").expect("write");

        assert_eq!(fingerprint_of(&first), fingerprint_of(&first));
        assert_eq!(fingerprint_of(&first), fingerprint_of(&second));

        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    #[test]
    fn fingerprint_changes_when_contents_or_files_change() {
        let dir = fixture_dir("changes");
        std::fs::write(dir.join("app.js"), "one").expect("write");
        let before = fingerprint_of(&dir);

        std::fs::write(dir.join("app.js"), "two").expect("write");
        let after_edit = fingerprint_of(&dir);
        assert_ne!(before, after_edit);

        std::fs::write(dir.join("extra.css"), "x").expect("write");
        assert_ne!(after_edit, fingerprint_of(&dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fingerprint_is_sixteen_lowercase_hex_characters() {
        let dir = fixture_dir("shape");
        std::fs::write(dir.join("a.txt"), "a").expect("write");
        let fingerprint = fingerprint_of(&dir);
        assert_eq!(fingerprint.len(), 16);
        assert!(
            fingerprint
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
