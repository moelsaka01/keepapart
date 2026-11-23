//! Profiles and the on-disk store.
//!
//! Layout under `~/.keepapart` (or `$KEEPAPART_HOME`):
//!
//! ```text
//! config.toml                 global default profile
//! profiles/<id>/profile.toml  name, tools, shared items
//! profiles/<id>/last_used     unix seconds, best effort
//! profiles/<id>/claude/       CLAUDE_CONFIG_DIR for this profile
//! profiles/<id>/codex/        CODEX_HOME for this profile
//! shared/claude/<item>        items shared between profiles
//! shared/codex/<item>
//! ```
//!
//! `<id>` is the name the profile was created with and never changes. `rename`
//! only changes the display name, so the tool directories keep the same path.
//! That matters: Claude Code keys its macOS Keychain entry to the config
//! directory, and Codex keys its keyring entry to a hash of `CODEX_HOME`, so
//! moving the directory would silently log the profile out.

use crate::paths::{self, validate_name, write_atomic};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub use crate::tools::Tool;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkMode {
    Symlink,
    Junction,
    Copy,
}

impl LinkMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkMode::Symlink => "symlink",
            LinkMode::Junction => "junction",
            LinkMode::Copy => "copy",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedEntry {
    /// `claude:skills`, `codex:prompts`, ...
    pub item: String,
    pub mode: LinkMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileFile {
    pub name: String,
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub shared: Vec<SharedEntry>,
    /// MCP servers keepapart added to this profile with `claude mcp add-json`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_synced: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub id: String,
    pub dir: PathBuf,
    pub file: ProfileFile,
}

impl Profile {
    pub fn name(&self) -> &str {
        &self.file.name
    }

    pub fn has(&self, tool: Tool) -> bool {
        self.file.tools.contains(&tool)
    }

    /// The directory handed to the official CLI as CLAUDE_CONFIG_DIR / CODEX_HOME.
    pub fn tool_dir(&self, tool: Tool) -> PathBuf {
        self.dir.join(tool.as_str())
    }

    pub fn tools_label(&self) -> String {
        self.file.tools.iter().map(|t| t.as_str()).collect::<Vec<_>>().join("+")
    }

    pub fn last_used(&self) -> Option<u64> {
        fs::read_to_string(self.dir.join("last_used")).ok()?.trim().parse().ok()
    }

    /// Best effort: two profiles launching at once must never fail on this.
    pub fn touch(&self) {
        let _ = write_atomic(&self.dir.join("last_used"), paths::now_secs().to_string().as_bytes());
    }

    pub fn shared_entry(&self, item: &str) -> Option<&SharedEntry> {
        self.file.shared.iter().find(|e| e.item == item)
    }

    pub fn save(&self) -> Result<()> {
        let text = toml::to_string_pretty(&self.file)?;
        write_atomic(&self.dir.join("profile.toml"), text.as_bytes())
    }

    /// Does one of the official CLI's login files exist? Existence only: the
    /// file is never opened.
    pub fn login_file_present(&self, tool: Tool) -> bool {
        tool.spec().login_files.iter().any(|f| self.tool_dir(tool).join(f).exists())
    }

    /// What keepapart can honestly say about a login without reading it.
    pub fn login_state(&self, tool: Tool) -> LoginState {
        if self.login_file_present(tool) {
            LoginState::Yes
        } else if tool.spec().login_files.is_empty() || (tool.spec().macos_keychain && cfg!(target_os = "macos")) {
            LoginState::Unknown
        } else {
            LoginState::No
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginState {
    Yes,
    No,
    /// The tool keeps it somewhere keepapart doesn't look (macOS Keychain) or
    /// doesn't say which file it uses.
    Unknown,
}

impl LoginState {
    pub fn label(self) -> &'static str {
        match self {
            LoginState::Yes => "yes",
            LoginState::No => "no",
            LoginState::Unknown => "?",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

pub struct Store {
    pub home: PathBuf,
}

impl Store {
    /// Open the store at `$KEEPAPART_HOME` or `~/.keepapart`, creating it.
    pub fn open() -> Result<Store> {
        Store::at(paths::keepapart_home()?)
    }

    pub fn at(home: PathBuf) -> Result<Store> {
        fs::create_dir_all(home.join("profiles")).with_context(|| format!("creating {}", home.display()))?;
        crate::tools::init(Some(&home))?;
        fs::create_dir_all(home.join("shared"))?;
        Ok(Store { home })
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.home.join("profiles")
    }

    pub fn shared_dir(&self, tool: Tool) -> PathBuf {
        let d = self.home.join("shared").join(tool.as_str());
        let _ = fs::create_dir_all(&d);
        d
    }

    pub fn config_path(&self) -> PathBuf {
        self.home.join("config.toml")
    }

    pub fn config(&self) -> Result<GlobalConfig> {
        match fs::read_to_string(self.config_path()) {
            Ok(s) => toml::from_str(&s).context("reading keepapart config.toml"),
            Err(_) => Ok(GlobalConfig::default()),
        }
    }

    pub fn save_config(&self, cfg: &GlobalConfig) -> Result<()> {
        write_atomic(&self.config_path(), toml::to_string_pretty(cfg)?.as_bytes())
    }

    fn load(&self, dir: &Path) -> Result<Profile> {
        let id = dir.file_name().and_then(|n| n.to_str()).context("bad profile dir")?.to_string();
        let text = fs::read_to_string(dir.join("profile.toml"))
            .with_context(|| format!("reading {}", dir.join("profile.toml").display()))?;
        let file: ProfileFile =
            toml::from_str(&text).with_context(|| format!("parsing {}", dir.join("profile.toml").display()))?;
        Ok(Profile { id, dir: dir.to_path_buf(), file })
    }

    pub fn list(&self) -> Result<Vec<Profile>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.profiles_dir())? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path().join("profile.toml").is_file() {
                out.push(self.load(&entry.path())?);
            }
        }
        out.sort_by(|a, b| a.file.name.cmp(&b.file.name));
        Ok(out)
    }

    pub fn try_find(&self, name: &str) -> Result<Option<Profile>> {
        Ok(self.list()?.into_iter().find(|p| p.file.name == name))
    }

    pub fn find(&self, name: &str) -> Result<Profile> {
        if let Some(p) = self.try_find(name)? {
            return Ok(p);
        }
        let names: Vec<String> = self.list()?.into_iter().map(|p| p.file.name).collect();
        if names.is_empty() {
            bail!(
                "no profile named {name:?}; you have no profiles yet. Create one: keepapart add {name} --tool claude"
            );
        }
        bail!("no profile named {name:?}. Profiles: {}", names.join(", "));
    }

    pub fn create(&self, name: &str, tools: &[Tool]) -> Result<Profile> {
        validate_name(name)?;
        if tools.is_empty() {
            bail!("a profile needs at least one tool");
        }
        if self.try_find(name)?.is_some() {
            bail!("profile {name:?} already exists");
        }
        // A renamed profile may still own the folder of this name.
        let mut id = name.to_string();
        let mut n = 2;
        while paths::exists_no_follow(&self.profiles_dir().join(&id)) {
            id = format!("{name}-{n}");
            n += 1;
        }
        let dir = self.profiles_dir().join(&id);
        fs::create_dir_all(&dir)?;
        let mut tools = tools.to_vec();
        tools.sort();
        tools.dedup();
        let p = Profile {
            id,
            dir,
            file: ProfileFile {
                name: name.to_string(),
                tools,
                created: paths::now_secs(),
                shared: vec![],
                mcp_synced: vec![],
            },
        };
        for t in &p.file.tools {
            // CODEX_HOME must exist before codex runs; create both up front.
            fs::create_dir_all(p.tool_dir(*t))?;
        }
        p.save()?;
        Ok(p)
    }

    /// Add a tool to an existing profile (e.g. a claude profile that now needs codex).
    pub fn add_tool(&self, p: &mut Profile, tool: Tool) -> Result<bool> {
        if p.has(tool) {
            return Ok(false);
        }
        fs::create_dir_all(p.tool_dir(tool))?;
        p.file.tools.push(tool);
        p.file.tools.sort();
        p.save()?;
        Ok(true)
    }

    pub fn rename(&self, old: &str, new: &str) -> Result<Profile> {
        validate_name(new)?;
        let mut p = self.find(old)?;
        if self.try_find(new)?.is_some() {
            bail!("profile {new:?} already exists");
        }
        p.file.name = new.to_string();
        p.save()?;
        let mut cfg = self.config()?;
        if cfg.default.as_deref() == Some(old) {
            cfg.default = Some(new.to_string());
            self.save_config(&cfg)?;
        }
        Ok(p)
    }

    /// Delete a profile folder, its login files included. Links to shared items
    /// are removed first so nothing outside the profile can be touched.
    pub fn remove(&self, name: &str) -> Result<Profile> {
        let p = self.find(name)?;
        let root = fs::canonicalize(self.profiles_dir())?;
        let dir = fs::canonicalize(&p.dir)?;
        if !dir.starts_with(&root) || dir == root {
            bail!("refusing to delete {}: not inside {}", dir.display(), root.display());
        }
        crate::share::unlink_all(&p)?;
        fs::remove_dir_all(&p.dir).with_context(|| format!("deleting {}", p.dir.display()))?;
        let mut cfg = self.config()?;
        if cfg.default.as_deref() == Some(name) {
            cfg.default = None;
            self.save_config(&cfg)?;
        }
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let t = tempfile::tempdir().unwrap();
        let s = Store::at(t.path().join("kp")).unwrap();
        (t, s)
    }

    #[test]
    fn each_profile_gets_its_own_tool_dirs() {
        let (_t, s) = store();
        let a = s.create("work", &[Tool::CLAUDE, Tool::CODEX]).unwrap();
        let b = s.create("personal", &[Tool::CLAUDE, Tool::CODEX]).unwrap();
        for t in [Tool::CLAUDE, Tool::CODEX] {
            assert!(a.tool_dir(t).is_dir());
            assert!(b.tool_dir(t).is_dir());
            assert_ne!(a.tool_dir(t), b.tool_dir(t));
            assert!(!a.tool_dir(t).starts_with(b.dir.as_path()));
        }
        assert_ne!(a.tool_dir(Tool::CLAUDE), a.tool_dir(Tool::CODEX));
    }

    #[test]
    fn duplicate_and_invalid_names_rejected() {
        let (_t, s) = store();
        s.create("work", &[Tool::CLAUDE]).unwrap();
        assert!(s.create("work", &[Tool::CODEX]).is_err());
        assert!(s.create("Work", &[Tool::CODEX]).is_err());
        assert!(s.create("../evil", &[Tool::CODEX]).is_err());
        assert!(s.create("ok", &[]).is_err());
    }

    #[test]
    fn rename_keeps_the_directory() {
        let (_t, s) = store();
        let before = s.create("acme", &[Tool::CLAUDE]).unwrap();
        s.save_config(&GlobalConfig { default: Some("acme".into()) }).unwrap();
        let after = s.rename("acme", "acme-corp").unwrap();
        assert_eq!(before.dir, after.dir);
        assert_eq!(before.tool_dir(Tool::CLAUDE), after.tool_dir(Tool::CLAUDE));
        assert!(s.try_find("acme").unwrap().is_none());
        assert_eq!(s.find("acme-corp").unwrap().id, "acme");
        assert_eq!(s.config().unwrap().default.as_deref(), Some("acme-corp"));
        // the old name is free again, and gets a fresh folder
        let reused = s.create("acme", &[Tool::CODEX]).unwrap();
        assert_eq!(reused.id, "acme-2");
        assert_ne!(reused.dir, after.dir);
    }

    #[test]
    fn remove_deletes_only_that_profile() {
        let (_t, s) = store();
        let a = s.create("a", &[Tool::CLAUDE]).unwrap();
        let b = s.create("b", &[Tool::CLAUDE]).unwrap();
        fs::write(b.tool_dir(Tool::CLAUDE).join("x"), "keep").unwrap();
        s.save_config(&GlobalConfig { default: Some("a".into()) }).unwrap();
        s.remove("a").unwrap();
        assert!(!a.dir.exists());
        assert!(b.tool_dir(Tool::CLAUDE).join("x").exists());
        assert_eq!(s.config().unwrap().default, None);
    }

    #[test]
    fn touch_and_last_used() {
        let (_t, s) = store();
        let a = s.create("a", &[Tool::CODEX]).unwrap();
        assert_eq!(a.last_used(), None);
        a.touch();
        assert!(a.last_used().is_some());
    }
}
