//! `keepapart export-config`: settings only, never logins.
//!
//! Only catalog items are exported (skills, agents, commands, instructions,
//! settings). settings.json and config.toml are parsed and every value that
//! looks secret is replaced with "<redacted>"; a file that can't be parsed is
//! skipped rather than exported unchecked.

use crate::profile::Profile;
use crate::share::{copy_any, profile_path, Kind, CATALOG};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

pub const REDACTED: &str = "<redacted>";

const SECRET_KEY_PARTS: &[&str] =
    &["key", "token", "secret", "password", "passwd", "auth", "credential", "bearer", "cookie", "private"];
const SECRET_VALUE_PREFIXES: &[&str] =
    &["sk-", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "AKIA", "glpat-", "eyJ"];

/// Login files are never exported, even if one turns up inside a skills folder.
const LOGIN_FILE_NAMES: &[&str] = &[".credentials.json", "auth.json", ".claude.json"];

pub fn secret_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    SECRET_KEY_PARTS.iter().any(|p| k.contains(p))
}

pub fn secret_value(v: &str) -> bool {
    let v = v.trim();
    let v = v.strip_prefix("Bearer ").unwrap_or(v);
    SECRET_VALUE_PREFIXES.iter().any(|p| v.starts_with(p))
}

pub fn redact_json(v: &mut serde_json::Value, under_secret: bool) -> usize {
    use serde_json::Value;
    match v {
        Value::Object(m) => m.iter_mut().map(|(k, v)| redact_json(v, under_secret || secret_key(k))).sum(),
        Value::Array(a) => a.iter_mut().map(|v| redact_json(v, under_secret)).sum(),
        Value::String(s) if under_secret || secret_value(s) => {
            *v = Value::String(REDACTED.into());
            1
        }
        Value::Number(_) if under_secret => {
            *v = Value::String(REDACTED.into());
            1
        }
        _ => 0,
    }
}

pub fn redact_toml(v: &mut toml::Value, under_secret: bool) -> usize {
    use toml::Value;
    match v {
        Value::Table(t) => t.iter_mut().map(|(k, v)| redact_toml(v, under_secret || secret_key(k))).sum(),
        Value::Array(a) => a.iter_mut().map(|v| redact_toml(v, under_secret)).sum(),
        Value::String(s) if under_secret || secret_value(s) => {
            *v = Value::String(REDACTED.into());
            1
        }
        Value::Integer(_) if under_secret => {
            *v = Value::String(REDACTED.into());
            1
        }
        _ => 0,
    }
}

fn strip_login_files(dir: &Path) -> Result<usize> {
    let mut n = 0;
    if !dir.is_dir() {
        return Ok(0);
    }
    for e in fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            n += strip_login_files(&p)?;
        } else if LOGIN_FILE_NAMES.iter().any(|l| e.file_name().to_string_lossy().eq_ignore_ascii_case(l)) {
            fs::remove_file(&p)?;
            n += 1;
        }
    }
    Ok(n)
}

#[derive(Debug, Default)]
pub struct Report {
    pub exported: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub redactions: usize,
}

pub fn export(p: &Profile, out: &Path) -> Result<Report> {
    if out.exists() && fs::read_dir(out)?.next().is_some() {
        bail!("{} already exists and isn't empty", out.display());
    }
    fs::create_dir_all(out)?;
    let mut r = Report::default();
    for item in CATALOG.iter().filter(|i| p.has(i.tool)) {
        let src = profile_path(p, item);
        if !src.exists() {
            continue;
        }
        let dst = out.join(item.tool.as_str()).join(item.name);
        match (item.kind, item.name) {
            (Kind::File, "settings.json") => {
                let text = fs::read_to_string(&src)?;
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(mut v) => {
                        r.redactions += redact_json(&mut v, false);
                        fs::create_dir_all(dst.parent().unwrap())?;
                        fs::write(&dst, serde_json::to_string_pretty(&v)? + "\n")?;
                    }
                    Err(e) => {
                        r.skipped.push((item.id(), format!("not valid JSON ({e}); not exported unchecked")));
                        continue;
                    }
                }
            }
            (Kind::File, "config.toml") => {
                let text = fs::read_to_string(&src)?;
                match toml::from_str::<toml::Value>(&text) {
                    Ok(mut v) => {
                        r.redactions += redact_toml(&mut v, false);
                        fs::create_dir_all(dst.parent().unwrap())?;
                        fs::write(&dst, toml::to_string_pretty(&v)?)?;
                    }
                    Err(e) => {
                        r.skipped.push((item.id(), format!("not valid TOML ({e}); not exported unchecked")));
                        continue;
                    }
                }
            }
            _ => copy_any(&src, &dst).with_context(|| format!("exporting {}", item.id()))?,
        }
        r.exported.push(item.id());
    }
    let stripped = strip_login_files(out)?;
    if stripped > 0 {
        r.skipped.push(("login files".into(), format!("{stripped} file(s) named like a login were left out")));
    }
    let manifest = format!(
        "# keepapart export: settings only. No logins, tokens or session history.\nprofile = {:?}\ntools = [{}]\nkeepapart = {:?}\nitems = [{}]\n",
        p.name(),
        p.file.tools.iter().map(|t| format!("{:?}", t.as_str())).collect::<Vec<_>>().join(", "),
        env!("CARGO_PKG_VERSION"),
        r.exported.iter().map(|i| format!("{i:?}")).collect::<Vec<_>>().join(", "),
    );
    fs::write(out.join("keepapart-export.toml"), manifest)?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_redaction() {
        let mut v: serde_json::Value = serde_json::json!({
            "model": "opus",
            "apiKeyHelper": "/bin/get-key",
            "env": { "ANTHROPIC_API_KEY": "sk-ant-123", "DEBUG": "1", "MY_TOKEN": 42 },
            "permissions": { "allow": ["Bash(ls)"] },
            "hooks": { "x": [{ "command": "echo ghp_abcdef" }] },
            "statusLine": { "command": "sk-live-in-a-value" }
        });
        let n = redact_json(&mut v, false);
        assert_eq!(v["model"], "opus");
        assert_eq!(v["apiKeyHelper"], REDACTED);
        assert_eq!(v["env"]["ANTHROPIC_API_KEY"], REDACTED);
        assert_eq!(v["env"]["MY_TOKEN"], REDACTED);
        assert_eq!(v["env"]["DEBUG"], "1");
        assert_eq!(v["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(v["statusLine"]["command"], REDACTED);
        assert_eq!(n, 4);
    }

    #[test]
    fn toml_redaction() {
        let mut v: toml::Value = toml::from_str(
            r#"
model = "gpt-5"
[mcp_servers.github]
command = "npx"
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_x", LOG = "info" }
[mcp_servers.remote]
url = "https://example.com"
http_headers = { Authorization = "Bearer abc" }
bearer_token_env_var = "REMOTE_TOKEN"
"#,
        )
        .unwrap();
        redact_toml(&mut v, false);
        let s = toml::to_string(&v).unwrap();
        assert!(s.contains("gpt-5"));
        assert!(s.contains("npx"));
        assert!(s.contains("LOG = \"info\""));
        assert!(!s.contains("ghp_x"));
        assert!(!s.contains("Bearer abc"));
    }
}
