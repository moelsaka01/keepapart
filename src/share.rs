//! Shared vs per-profile config.
//!
//! Only items in [`CATALOG`] can ever be shared. Logins and session history are
//! not in the catalog, so there is no code path that links or copies them.
//! A shared item lives once in `shared/<tool>/<item>`, and each profile that
//! shares it gets a link at `profiles/<id>/<tool>/<item>`:
//!
//! * macOS / Linux: a symlink.
//! * Windows: a symlink if allowed (Developer Mode or admin), otherwise a
//!   junction for folders (needs no special rights) or a copy for files.
//!
//! `KEEPAPART_LINK_MODE=copy` forces copies everywhere; `no-symlink` acts like
//! a Windows account without symlink rights (used by tests and CI).

use crate::paths::{exists_no_follow, now_secs, same_path};
use crate::profile::{LinkMode, Profile, SharedEntry, Store, Tool};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub tool: Tool,
    pub name: &'static str,
    pub kind: Kind,
    /// Shared automatically for new profiles (unless `--isolated`).
    pub default_shared: bool,
    pub about: &'static str,
}

impl Item {
    pub fn id(&self) -> String {
        format!("{}:{}", self.tool.as_str(), self.name)
    }
}

const fn item(tool: Tool, name: &'static str, kind: Kind, default_shared: bool, about: &'static str) -> Item {
    Item { tool, name, kind, default_shared, about }
}

/// Everything keepapart is willing to share. Nothing else, ever.
pub const CATALOG: &[Item] = &[
    item(Tool::Claude, "skills", Kind::Dir, true, "Agent skills"),
    item(Tool::Claude, "agents", Kind::Dir, true, "Subagent definitions"),
    item(Tool::Claude, "commands", Kind::Dir, true, "Custom slash commands"),
    item(Tool::Claude, "output-styles", Kind::Dir, true, "Output styles"),
    item(Tool::Claude, "CLAUDE.md", Kind::File, true, "Your global instructions"),
    item(Tool::Claude, "settings.json", Kind::File, false, "Settings: model, permissions, hooks, env, statusline"),
    item(Tool::Claude, "plugins", Kind::Dir, false, "Plugins and marketplaces (can bundle MCP servers)"),
    item(Tool::Codex, "skills", Kind::Dir, true, "Agent skills"),
    item(Tool::Codex, "prompts", Kind::Dir, true, "Custom prompts"),
    item(Tool::Codex, "AGENTS.md", Kind::File, true, "Your global instructions"),
    item(Tool::Codex, "config.toml", Kind::File, false, "Settings and MCP servers"),
    item(Tool::Codex, "rules", Kind::Dir, false, "Command approval rules"),
];

/// What the official CLIs keep per account. Never in the catalog; `doctor`
/// warns if any of these is a link.
pub const NEVER_SHARED: &[(Tool, &str)] = &[
    (Tool::Claude, ".credentials.json"),
    (Tool::Claude, ".claude.json"),
    (Tool::Claude, "projects"),
    (Tool::Claude, "history.jsonl"),
    (Tool::Claude, "sessions"),
    (Tool::Claude, "todos"),
    (Tool::Claude, "file-history"),
    (Tool::Claude, "shell-snapshots"),
    (Tool::Claude, "statsig"),
    (Tool::Codex, "auth.json"),
    (Tool::Codex, "sessions"),
    (Tool::Codex, "archived_sessions"),
    (Tool::Codex, "history.jsonl"),
    (Tool::Codex, "log"),
];

pub fn lookup(id: &str) -> Result<&'static Item> {
    let (tool, name) = id
        .split_once(':')
        .with_context(|| format!("item must look like claude:skills or codex:prompts, got {id:?}"))?;
    let tool = Tool::parse(tool).with_context(|| format!("unknown tool {tool:?} in {id:?}"))?;
    if NEVER_SHARED.iter().any(|(t, n)| *t == tool && n.eq_ignore_ascii_case(name)) {
        bail!("{id} holds a login or session history and is always kept per profile");
    }
    CATALOG.iter().find(|i| i.tool == tool && i.name == name).with_context(|| {
        let ids: Vec<String> = CATALOG.iter().filter(|i| i.tool == tool).map(|i| i.id()).collect();
        format!("{id:?} can't be shared. Shareable: {}", ids.join(", "))
    })
}

fn link_mode_env(want: &str) -> bool {
    std::env::var("KEEPAPART_LINK_MODE").map(|v| v.eq_ignore_ascii_case(want)).unwrap_or(false)
}

fn forced_copy() -> bool {
    link_mode_env("copy")
}

/// `KEEPAPART_LINK_MODE=no-symlink` behaves like a Windows account without
/// symlink rights: junctions for folders, copies for files (copies on Unix).
fn symlinks_disabled() -> bool {
    link_mode_env("no-symlink")
}

/// Is `path` a symlink or (on Windows) a junction?
pub fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

/// Remove a link without touching what it points to.
pub fn remove_link(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        let ft = fs::symlink_metadata(path)?.file_type();
        if ft.is_symlink_dir() {
            // Covers directory symlinks and junctions; never recurses.
            return fs::remove_dir(path).with_context(|| format!("removing link {}", path.display()));
        }
    }
    fs::remove_file(path).with_context(|| format!("removing link {}", path.display()))
}

/// Create `link` pointing at `target`, using the best mode the OS allows.
pub fn create_link(target: &Path, link: &Path, kind: Kind) -> Result<LinkMode> {
    if forced_copy() || (cfg!(unix) && symlinks_disabled()) {
        copy_any(target, link)?;
        return Ok(LinkMode::Copy);
    }
    #[cfg(unix)]
    {
        let _ = kind;
        std::os::unix::fs::symlink(target, link)
            .with_context(|| format!("linking {} -> {}", link.display(), target.display()))?;
        Ok(LinkMode::Symlink)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let try_symlink = !symlinks_disabled();
        match kind {
            Kind::Dir => {
                if try_symlink && symlink_dir(target, link).is_ok() {
                    return Ok(LinkMode::Symlink);
                }
                if junction::create(target, link).is_ok() {
                    return Ok(LinkMode::Junction);
                }
            }
            Kind::File => {
                if try_symlink && symlink_file(target, link).is_ok() {
                    return Ok(LinkMode::Symlink);
                }
            }
        }
        copy_any(target, link)?;
        Ok(LinkMode::Copy)
    }
}

/// Copy a file or a folder tree. Links inside a folder are skipped, so a copy
/// can never loop or pull in something outside the tree.
pub fn copy_any(src: &Path, dst: &Path) -> Result<()> {
    let meta = fs::metadata(src).with_context(|| format!("reading {}", src.display()))?;
    if meta.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                continue;
            }
            copy_any(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(src, dst).with_context(|| format!("copying {} to {}", src.display(), dst.display()))?;
        // Keep the source's modified time. Windows does this already; Linux
        // and macOS stamp the copy "now", which would make newest-wins sync
        // treat an old edit as the latest one.
        if let Ok(t) = meta.modified() {
            if let Ok(f) = fs::OpenOptions::new().write(true).open(dst) {
                let _ = f.set_modified(t);
            }
        }
    }
    Ok(())
}

fn remove_any(path: &Path) -> Result<()> {
    if is_link(path) {
        return remove_link(path);
    }
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if exists_no_follow(path) {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn move_any(src: &Path, dst: &Path) -> Result<()> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_any(src, dst)?;
    remove_any(src)
}

fn create_empty(path: &Path, kind: Kind) -> Result<()> {
    match kind {
        Kind::Dir => fs::create_dir_all(path)?,
        Kind::File => {
            if let Some(p) = path.parent() {
                fs::create_dir_all(p)?;
            }
            let body = if path.extension().is_some_and(|e| e == "json") { "{}\n" } else { "" };
            fs::write(path, body)?;
        }
    }
    Ok(())
}

pub fn shared_path(store: &Store, item: &Item) -> PathBuf {
    store.shared_dir(item.tool).join(item.name)
}

pub fn profile_path(p: &Profile, item: &Item) -> PathBuf {
    p.tool_dir(item.tool).join(item.name)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ShareOutcome {
    Linked(LinkMode),
    AlreadyShared(LinkMode),
    /// The profile's own copy became the shared copy (first profile to share it).
    Seeded(LinkMode),
    /// The profile's own copy was moved aside to this backup.
    BackedUp(LinkMode, PathBuf),
}

/// Share `item` for profile `p`. If both the profile and the shared store
/// already have different content, refuse unless `force`, in which case the
/// profile's copy is renamed to a timestamped backup (never deleted).
pub fn share(store: &Store, p: &mut Profile, item: &Item, force: bool) -> Result<ShareOutcome> {
    if !p.has(item.tool) {
        bail!("profile {:?} doesn't use {}", p.name(), item.tool.as_str());
    }
    let target = shared_path(store, item);
    let link = profile_path(p, item);
    let id = item.id();

    if let Some(e) = p.shared_entry(&id).cloned() {
        match e.mode {
            LinkMode::Copy => {
                // Refresh the copy from the shared store.
                if target.exists() {
                    remove_any(&link)?;
                    copy_any(&target, &link)?;
                }
                return Ok(ShareOutcome::AlreadyShared(LinkMode::Copy));
            }
            _ if is_link(&link) && same_path(&link, &target) => return Ok(ShareOutcome::AlreadyShared(e.mode)),
            _ => {} // record is stale; fall through and repair
        }
    }

    let mut outcome_kind = 0u8; // 0 linked, 1 seeded, 2 backed up
    let mut backup = None;
    if is_link(&link) {
        if same_path(&link, &target) {
            // Linked by hand or by an older run: just record it.
        } else {
            bail!(
                "{} is a link to somewhere else ({}); remove it yourself if that's intended",
                link.display(),
                fs::read_link(&link).map(|t| t.display().to_string()).unwrap_or_default()
            );
        }
    } else if exists_no_follow(&link) {
        if !exists_no_follow(&target) {
            move_any(&link, &target)?;
            outcome_kind = 1;
        } else if !force {
            bail!(
                "both this profile and the shared store have {id}.\n  profile: {}\n  shared:  {}\nRun again with --force to use the shared one (this profile's copy is kept as a backup).",
                link.display(),
                target.display()
            );
        } else {
            let b = link.with_file_name(format!("{}.keepapart-backup-{}", item.name, now_secs()));
            move_any(&link, &b)?;
            backup = Some(b);
            outcome_kind = 2;
        }
    } else if !exists_no_follow(&target) {
        create_empty(&target, item.kind)?;
    }

    let mode = if is_link(&link) {
        if cfg!(windows) && fs::symlink_metadata(&link).is_ok() && junction_like(&link) {
            LinkMode::Junction
        } else {
            LinkMode::Symlink
        }
    } else {
        create_link(&target, &link, item.kind)?
    };

    p.file.shared.retain(|e| e.item != id);
    p.file.shared.push(SharedEntry { item: id, mode });
    p.file.shared.sort_by(|a, b| a.item.cmp(&b.item));
    p.save()?;

    Ok(match (outcome_kind, backup) {
        (1, _) => ShareOutcome::Seeded(mode),
        (2, Some(b)) => ShareOutcome::BackedUp(mode, b),
        _ => ShareOutcome::Linked(mode),
    })
}

#[cfg(windows)]
fn junction_like(path: &Path) -> bool {
    junction::exists(path).unwrap_or(false)
}
#[cfg(not(windows))]
fn junction_like(_: &Path) -> bool {
    false
}

/// Stop sharing: the profile gets its own private copy of the current shared
/// content, so nothing it uses disappears.
pub fn unshare(store: &Store, p: &mut Profile, item: &Item) -> Result<()> {
    let id = item.id();
    let target = shared_path(store, item);
    let link = profile_path(p, item);
    let entry = p.shared_entry(&id).cloned();
    if entry.is_none() && !(is_link(&link) && same_path(&link, &target)) {
        bail!("{id} isn't shared for profile {:?}", p.name());
    }
    if is_link(&link) {
        remove_link(&link)?;
        if target.exists() {
            copy_any(&target, &link)?;
        }
    }
    // Copy mode: the profile already holds a private copy.
    p.file.shared.retain(|e| e.item != id);
    p.save()
}

/// Remove every link to the shared store inside a profile (used before deleting it).
pub fn unlink_all(p: &Profile) -> Result<()> {
    for item in CATALOG {
        let link = profile_path(p, item);
        if is_link(&link) {
            remove_link(&link)?;
        }
    }
    Ok(())
}

/// Share every default item for a fresh profile.
pub fn apply_defaults(store: &Store, p: &mut Profile) -> Result<Vec<(String, Result<ShareOutcome>)>> {
    let items: Vec<&Item> = CATALOG.iter().filter(|i| i.default_shared && p.has(i.tool)).collect();
    let mut out = Vec::new();
    for item in items {
        out.push((item.id(), share(store, p, item, false)));
    }
    Ok(out)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ItemState {
    Shared(LinkMode),
    /// Recorded as shared but the link is missing or points elsewhere.
    Broken,
    Private,
    Absent,
}

pub fn state(store: &Store, p: &Profile, item: &Item) -> ItemState {
    let link = profile_path(p, item);
    let target = shared_path(store, item);
    match p.shared_entry(&item.id()) {
        Some(e) if e.mode == LinkMode::Copy => {
            if link.exists() {
                ItemState::Shared(LinkMode::Copy)
            } else {
                ItemState::Broken
            }
        }
        Some(e) => {
            if is_link(&link) && same_path(&link, &target) && target.exists() {
                ItemState::Shared(e.mode)
            } else {
                ItemState::Broken
            }
        }
        None if exists_no_follow(&link) => ItemState::Private,
        None => ItemState::Absent,
    }
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Two-way sync of one file: if the contents differ, the newer one wins.
fn sync_file(a: &Path, b: &Path) -> Result<bool> {
    match (a.is_file(), b.is_file()) {
        (true, false) => copy_any(a, b).map(|_| true),
        (false, true) => copy_any(b, a).map(|_| true),
        (true, true) => {
            if fs::read(a)? == fs::read(b)? {
                return Ok(false);
            }
            if mtime(a) >= mtime(b) {
                copy_any(a, b)?;
            } else {
                copy_any(b, a)?;
            }
            Ok(true)
        }
        (false, false) => Ok(false),
    }
}

/// Two-way sync of a folder tree, newest file wins. Deletions aren't
/// propagated (a file missing on one side is copied back), and links are skipped.
fn sync_tree(a: &Path, b: &Path) -> Result<bool> {
    fs::create_dir_all(a)?;
    fs::create_dir_all(b)?;
    let mut names = std::collections::BTreeSet::new();
    for dir in [a, b] {
        for e in fs::read_dir(dir)? {
            let e = e?;
            if !e.file_type()?.is_symlink() {
                names.insert(e.file_name());
            }
        }
    }
    let mut changed = false;
    for n in names {
        let (x, y) = (a.join(&n), b.join(&n));
        if x.is_dir() || y.is_dir() {
            changed |= sync_tree(&x, &y)?;
        } else {
            changed |= sync_file(&x, &y)?;
        }
    }
    Ok(changed)
}

/// Bring a profile's copied (not linked) shared items and the shared store
/// level with each other, newest wins. Runs before every `keepapart run`, so
/// copies on Windows without symlink rights stay current at each launch.
pub fn sync_copies(store: &Store, p: &Profile) -> Result<Vec<String>> {
    let mut changed = Vec::new();
    for e in p.file.shared.iter().filter(|e| e.mode == LinkMode::Copy) {
        let item = lookup(&e.item)?;
        let (mine, shared) = (profile_path(p, item), shared_path(store, item));
        let did = match item.kind {
            Kind::File => sync_file(&mine, &shared)?,
            Kind::Dir => sync_tree(&mine, &shared)?,
        };
        if did {
            changed.push(e.item.clone());
        }
    }
    Ok(changed)
}

/// Sync every profile's copies. Two passes, so an edit made in one profile
/// reaches all the others.
pub fn sync_all(store: &Store) -> Result<Vec<(String, Vec<String>)>> {
    let profiles = store.list()?;
    let mut report: Vec<(String, Vec<String>)> = Vec::new();
    for _ in 0..2 {
        for p in &profiles {
            let ch = sync_copies(store, p)?;
            if !ch.is_empty() {
                match report.iter_mut().find(|(n, _)| n == p.name()) {
                    Some((_, v)) => v.extend(ch),
                    None => report.push((p.name().to_string(), ch)),
                }
            }
        }
    }
    for (_, v) in report.iter_mut() {
        v.sort();
        v.dedup();
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_never_contains_logins_or_history() {
        for i in CATALOG {
            assert!(
                !NEVER_SHARED.iter().any(|(t, n)| *t == i.tool && *n == i.name),
                "{} must never be shareable",
                i.id()
            );
        }
        assert!(lookup("claude:.credentials.json").is_err());
        assert!(lookup("codex:auth.json").is_err());
        assert!(lookup("codex:AUTH.JSON").is_err());
        assert!(lookup("claude:projects").is_err());
        assert!(lookup("claude:../../etc").is_err());
        assert!(lookup("skills").is_err());
        assert!(lookup("gemini:skills").is_err());
        assert_eq!(lookup("claude:skills").unwrap().kind, Kind::Dir);
    }
}
