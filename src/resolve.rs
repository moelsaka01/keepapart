//! Which profile applies here?
//!
//! Order: an explicit name, then `$KEEPAPART_PROFILE`, then the nearest
//! `.keepapart` file walking up from the current folder, then the global
//! default set with `keepapart use <name> --global`.

use crate::paths::{validate_name, write_atomic, MARKER_FILE};
use crate::profile::Store;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Explicit,
    Env,
    Marker(PathBuf),
    Global,
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Source::Explicit => "named on the command line".into(),
            Source::Env => "from $KEEPAPART_PROFILE".into(),
            Source::Marker(p) => format!("from {}", p.display()),
            Source::Global => "global default".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub name: String,
    pub source: Source,
}

/// Read a `.keepapart` file: the first line that isn't blank or a `#` comment.
pub fn read_marker(path: &Path) -> Result<String> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let name = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .with_context(|| format!("{} is empty", path.display()))?
        .to_string();
    validate_name(&name).with_context(|| format!("in {}", path.display()))?;
    Ok(name)
}

/// The nearest `.keepapart` *file* at or above `start`. A folder of that name
/// (keepapart's own data dir in your home folder) is ignored.
pub fn find_marker(start: &Path) -> Option<PathBuf> {
    start.ancestors().map(|d| d.join(MARKER_FILE)).find(|p| p.is_file())
}

pub fn write_marker(dir: &Path, name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    let path = dir.join(MARKER_FILE);
    if path.is_dir() {
        bail!(
            "{} is a folder (keepapart's own data, if this is your home folder); can't pin a profile here. Use `keepapart use {name} --global` instead",
            path.display()
        );
    }
    let body = format!(
        "{name}\n# keepapart: claude/codex in this folder use the {name:?} profile.\n# Safe to commit: it holds a profile name, never a login.\n"
    );
    write_atomic(&path, body.as_bytes())?;
    Ok(path)
}

pub fn resolve(store: &Store, explicit: Option<&str>, cwd: &Path) -> Result<Option<Resolved>> {
    if let Some(n) = explicit {
        return Ok(Some(Resolved { name: n.to_string(), source: Source::Explicit }));
    }
    if let Ok(n) = std::env::var("KEEPAPART_PROFILE") {
        let n = n.trim().to_string();
        if !n.is_empty() {
            return Ok(Some(Resolved { name: n, source: Source::Env }));
        }
    }
    if let Some(m) = find_marker(cwd) {
        return Ok(Some(Resolved { name: read_marker(&m)?, source: Source::Marker(m) }));
    }
    if let Some(n) = store.config()?.default {
        return Ok(Some(Resolved { name: n, source: Source::Global }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::GlobalConfig;

    #[test]
    fn nearest_marker_wins_and_dirs_are_ignored() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let deep = root.join("client").join("repo").join("src");
        fs::create_dir_all(&deep).unwrap();
        // keepapart's own data folder is also called .keepapart: must be skipped
        fs::create_dir_all(root.join(MARKER_FILE)).unwrap();
        assert_eq!(find_marker(&deep), None);

        assert!(write_marker(root, "work").is_err());
        write_marker(&root.join("client"), "client-acme").unwrap();
        let m = find_marker(&deep).unwrap();
        assert_eq!(read_marker(&m).unwrap(), "client-acme");

        write_marker(&root.join("client").join("repo"), "work").unwrap();
        assert_eq!(read_marker(&find_marker(&deep).unwrap()).unwrap(), "work");
    }

    #[test]
    fn marker_parsing() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join(MARKER_FILE);
        fs::write(&p, "\n# comment\n  personal  \r\nignored\n").unwrap();
        assert_eq!(read_marker(&p).unwrap(), "personal");
        fs::write(&p, "# only comments\n").unwrap();
        assert!(read_marker(&p).is_err());
        fs::write(&p, "../../etc\n").unwrap();
        assert!(read_marker(&p).is_err());
    }

    #[test]
    fn precedence() {
        let t = tempfile::tempdir().unwrap();
        let store = Store::at(t.path().join("kp")).unwrap();
        let dir = t.path().join("proj");
        fs::create_dir_all(&dir).unwrap();
        // KEEPAPART_PROFILE is process-global; this test assumes it's unset.
        if std::env::var_os("KEEPAPART_PROFILE").is_some() {
            return;
        }
        assert_eq!(resolve(&store, None, &dir).unwrap(), None);
        store.save_config(&GlobalConfig { default: Some("personal".into()) }).unwrap();
        assert_eq!(resolve(&store, None, &dir).unwrap().unwrap().source, Source::Global);
        write_marker(&dir, "work").unwrap();
        let r = resolve(&store, None, &dir).unwrap().unwrap();
        assert_eq!(r.name, "work");
        assert!(matches!(r.source, Source::Marker(_)));
        let r = resolve(&store, Some("x"), &dir).unwrap().unwrap();
        assert_eq!((r.name.as_str(), r.source), ("x", Source::Explicit));
    }
}
