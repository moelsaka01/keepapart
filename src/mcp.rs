//! Shared MCP servers for Claude Code.
//!
//! Claude Code keeps user-level MCP servers in `.claude.json`, next to account
//! data, so that file is never linked or edited by keepapart. Instead the
//! shared list lives in `shared/claude/mcp-servers.json`, and `sync` installs
//! it into each profile through the official CLI:
//! `claude mcp add-json --scope user <name> <json>`.
//! (Codex keeps MCP servers in `config.toml`, which can be shared directly.)

use crate::launch;
use crate::paths::write_atomic;
use crate::profile::{Profile, Store, Tool};
use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

pub fn path(store: &Store) -> PathBuf {
    store.shared_dir(Tool::Claude).join("mcp-servers.json")
}

pub fn load(store: &Store) -> Result<Map<String, Value>> {
    match fs::read_to_string(path(store)) {
        Ok(s) => match serde_json::from_str::<Value>(&s).context("reading mcp-servers.json")? {
            Value::Object(m) => Ok(m),
            _ => bail!("{} must hold a JSON object of servers", path(store).display()),
        },
        Err(_) => Ok(Map::new()),
    }
}

fn save(store: &Store, m: &Map<String, Value>) -> Result<()> {
    write_atomic(&path(store), (serde_json::to_string_pretty(m)? + "\n").as_bytes())
}

pub fn validate_server_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        bail!("MCP server names may use letters, digits, '-', '_' and '.' (got {name:?})");
    }
    Ok(())
}

/// Add or replace a shared server. `json` is the same JSON `claude mcp add-json` takes.
pub fn add(store: &Store, name: &str, json: &str) -> Result<()> {
    validate_server_name(name)?;
    let v: Value = serde_json::from_str(json).context("the server config must be valid JSON")?;
    let Value::Object(o) = &v else { bail!("the server config must be a JSON object") };
    if !o.contains_key("command") && !o.contains_key("url") {
        bail!("the server config needs a \"command\" (stdio) or a \"url\" (http/sse)");
    }
    let mut m = load(store)?;
    m.insert(name.to_string(), v);
    save(store, &m)
}

pub fn remove(store: &Store, name: &str) -> Result<bool> {
    let mut m = load(store)?;
    let had = m.remove(name).is_some();
    save(store, &m)?;
    Ok(had)
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// Make a profile's user-level MCP servers match the shared list, through the
/// official CLI. Servers keepapart added earlier but no longer shares are
/// removed; servers you added yourself under other names are left alone.
pub fn sync_profile(store: &Store, p: &mut Profile) -> Result<Vec<String>> {
    if !p.has(Tool::Claude) {
        return Ok(vec![]);
    }
    let m = load(store)?;
    let mut log = Vec::new();
    for old in p.file.mcp_synced.clone() {
        if !m.contains_key(&old) {
            launch::capture(p, Tool::Claude, &os(&["mcp", "remove", "--scope", "user", &old]))?;
            log.push(format!("removed {old}"));
        }
    }
    let mut synced = Vec::new();
    for (name, cfg) in &m {
        // Replace, so edits to the shared config reach the profile.
        launch::capture(p, Tool::Claude, &os(&["mcp", "remove", "--scope", "user", name]))?;
        let json = serde_json::to_string(cfg)?;
        let out = launch::capture(p, Tool::Claude, &os(&["mcp", "add-json", "--scope", "user", name, &json]))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("claude mcp add-json {name} failed for profile {:?}: {}", p.name(), err.trim());
        }
        synced.push(name.clone());
        log.push(format!("added {name}"));
    }
    p.file.mcp_synced = synced;
    p.save()?;
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_validates_and_remove_reports() {
        let t = tempfile::tempdir().unwrap();
        let s = Store::at(t.path().join("kp")).unwrap();
        assert!(add(&s, "bad name", r#"{"command":"x"}"#).is_err());
        assert!(add(&s, "gh", "not json").is_err());
        assert!(add(&s, "gh", "[1]").is_err());
        assert!(add(&s, "gh", r#"{"args":[]}"#).is_err());
        add(&s, "gh", r#"{"command":"npx","args":["-y","@modelcontextprotocol/server-github"]}"#).unwrap();
        add(&s, "docs", r#"{"type":"http","url":"https://example.com/mcp"}"#).unwrap();
        let m = load(&s).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m["gh"]["command"], "npx");
        assert!(remove(&s, "gh").unwrap());
        assert!(!remove(&s, "gh").unwrap());
        assert_eq!(load(&s).unwrap().len(), 1);
    }
}
