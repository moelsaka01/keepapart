//! Where keepapart keeps its data, and the rules for profile names.

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Name of the per-directory file that pins a profile (`keepapart use <name>`).
pub const MARKER_FILE: &str = ".keepapart";

/// Root of keepapart's own data: `$KEEPAPART_HOME`, or `~/.keepapart`.
pub fn keepapart_home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("KEEPAPART_HOME") {
        if !h.is_empty() {
            return Ok(PathBuf::from(h));
        }
    }
    let home = dirs::home_dir().context("could not find your home directory; set KEEPAPART_HOME")?;
    Ok(home.join(".keepapart"))
}

/// Names that Windows refuses as file or folder names, whatever the extension.
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Profile names become folder names on every OS, so they are kept boring:
/// 1-40 chars of `a-z 0-9 - _`, starting with a letter or digit, and never a
/// name Windows reserves. Lowercase only, so `Work` and `work` can't collide on
/// case-insensitive file systems (Windows, default macOS).
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 40 {
        bail!("profile name must be 1-40 characters long");
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        bail!("profile name must start with a lowercase letter or a digit: {name:?}");
    }
    if let Some(bad) = name.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_'))
    {
        bail!("profile name may only use a-z, 0-9, '-' and '_' (found {bad:?} in {name:?})");
    }
    if WINDOWS_RESERVED.contains(&name) {
        bail!("{name:?} is a reserved device name on Windows; pick another name");
    }
    Ok(())
}

/// Write a file by writing a sibling temp file and renaming it over the target,
/// so a crash or a second keepapart process never sees a half-written file.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent")?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    {
        let mut f = fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        f.write_all(contents)?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// True when `path` exists in any form, including a dangling link.
pub fn exists_no_follow(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Compare two paths after resolving links, falling back to a plain compare.
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// "just now", "5m ago", "3h ago", "2d ago", or "never".
pub fn ago(then: Option<u64>, now: u64) -> String {
    let Some(then) = then else { return "never".into() };
    let d = now.saturating_sub(then);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", d / 60),
        3600..=86_399 => format!("{}h ago", d / 3600),
        _ => format!("{}d ago", d / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_names() {
        for n in ["work", "personal", "client-acme", "acme_2", "a", "9lives"] {
            validate_name(n).unwrap();
        }
    }

    #[test]
    fn rejects_names_that_break_paths() {
        for n in [
            "",
            "Work",
            "-work",
            "_x",
            "a/b",
            "a\\b",
            "..",
            ".",
            "a b",
            "a:b",
            "café",
            "x*",
            "con",
            "nul",
            "com1",
            "lpt9",
            "aux",
            &"a".repeat(41),
        ] {
            assert!(validate_name(n).is_err(), "{n:?} should be rejected");
        }
    }

    #[test]
    fn ago_formats() {
        assert_eq!(ago(None, 100), "never");
        assert_eq!(ago(Some(100), 130), "just now");
        assert_eq!(ago(Some(0), 600), "10m ago");
        assert_eq!(ago(Some(0), 7200), "2h ago");
        assert_eq!(ago(Some(0), 3 * 86_400), "3d ago");
        assert_eq!(ago(Some(200), 100), "just now");
    }

    #[test]
    fn atomic_write_replaces() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("sub").join("f.toml");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(fs::read_dir(t.path().join("sub")).unwrap().count(), 1);
    }
}
