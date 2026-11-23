//! Launching the official CLIs with a profile's environment.
//!
//! keepapart sets only the documented variables each tool reads to find its
//! folder (CLAUDE_CONFIG_DIR, CODEX_HOME, GEMINI_CLI_HOME, ...; see
//! [`crate::tools`]) plus KEEPAPART_ACTIVE, and then runs the real CLI. Each
//! launch is its own process with its own environment, so any number of
//! profiles can run side by side.

use crate::profile::{Profile, Tool};
use crate::tools::{self, Value};
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// `$KEEPAPART_<TOOL>_BIN`
    Override,
    /// On PATH (with PATHEXT on Windows, so `.cmd` shims from npm work too).
    Path,
    /// In the official native installers' folder (`~/.local/bin`), which isn't on PATH.
    InstallerDir,
}

/// `KEEPAPART_CLAUDE_BIN`, `KEEPAPART_GEMINI_BIN`, ...
pub fn bin_override_var(tool: Tool) -> String {
    format!("KEEPAPART_{}_BIN", tool.as_str().to_ascii_uppercase().replace('-', "_"))
}

/// The user's home folder, honoring `USERPROFILE` (Windows) / `HOME` first.
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).or_else(dirs::home_dir)
}

/// Native installers often put the CLI in `~/.local/bin`, which may not be on
/// PATH yet (the Claude Code installer says so on Windows).
pub fn installer_dir_binary(tool: Tool) -> Option<PathBuf> {
    let dir = home_dir()?.join(".local").join("bin");
    let name = if cfg!(windows) { format!("{}.exe", tool.program()) } else { tool.program().to_string() };
    let p = dir.join(name);
    p.is_file().then_some(p)
}

/// Find the official CLI: the override variable, then PATH, then `~/.local/bin`.
pub fn locate(tool: Tool) -> Result<(PathBuf, Found)> {
    if let Some(p) = std::env::var_os(bin_override_var(tool)).filter(|v| !v.is_empty()) {
        return Ok((PathBuf::from(p), Found::Override));
    }
    if let Ok(p) = which::which(tool.program()) {
        return Ok((p, Found::Path));
    }
    if let Some(p) = installer_dir_binary(tool) {
        return Ok((p, Found::InstallerDir));
    }
    let how = tool.spec().install;
    anyhow::bail!(
        "`{}` ({}) isn't installed or isn't on your PATH.{}",
        tool.program(),
        tool.spec().name,
        if how.is_empty() { String::new() } else { format!(" Install it first: {how}") }
    )
}

pub fn program_path(tool: Tool) -> Result<PathBuf> {
    locate(tool).map(|(p, _)| p)
}

/// The advice for a CLI found only in the installer's folder.
pub fn path_advice(tool: Tool, path: &std::path::Path) -> String {
    format!(
        "`{}` isn't on your PATH; keepapart uses {}. Plain `{}` won't work until you add {} to your PATH{}",
        tool.program(),
        path.display(),
        tool.program(),
        path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
        if cfg!(windows) { " (Start > \"Edit environment variables for your account\" > Path)" } else { "" },
    )
}

/// The variables one tool gets for this profile.
pub fn tool_env(p: &Profile, tool: Tool) -> Vec<(&'static str, OsString)> {
    let dir = p.tool_dir(tool);
    tool.spec()
        .env
        .iter()
        .map(|e| {
            let v: OsString = match e.value {
                Value::Dir("") => dir.clone().into_os_string(),
                Value::Dir(sub) => dir.join(sub).into_os_string(),
                Value::Lit(s) => s.into(),
            };
            (e.var, v)
        })
        .collect()
}

/// The environment a profile adds: every tool it uses (so a `claude` session
/// that shells out to `gemini` stays on the same profile), plus KEEPAPART_ACTIVE.
pub fn profile_env(p: &Profile) -> Vec<(&'static str, OsString)> {
    let mut env: Vec<(&'static str, OsString)> = p.file.tools.iter().flat_map(|t| tool_env(p, *t)).collect();
    env.push(("KEEPAPART_ACTIVE", p.name().into()));
    env
}

/// Variables that make the official CLI ignore the profile's login.
pub fn overriding_vars(tool: Tool) -> &'static [&'static str] {
    tool.spec().overriding
}

pub fn warn_overrides(tool: Tool, clean_env: bool) {
    if clean_env {
        return;
    }
    for v in overriding_vars(tool) {
        if std::env::var_os(v).is_some_and(|x| !x.is_empty()) {
            eprintln!(
                "keepapart: warning: ${v} is set in your environment; {} will use it instead of this profile's login \
                 (add --clean-env to hide it from this launch)",
                tool.as_str()
            );
        }
    }
}

/// Build the command. `clean_env` hides the variables that would override the
/// profile's login (API keys, OAuth tokens) from this one process.
pub fn command(p: &Profile, tool: Tool, args: &[OsString], clean_env: bool) -> Result<Command> {
    let prog = program_path(tool)?;
    let mut cmd = Command::new(prog);
    cmd.args(args);
    // Don't let a value the shell hook set for another profile's tool leak in.
    if std::env::var_os("KEEPAPART_ACTIVE").is_some() {
        for v in tools::hook_vars() {
            cmd.env_remove(v);
        }
    }
    for (k, v) in profile_env(p) {
        cmd.env(k, v);
    }
    if clean_env {
        for t in tools::all() {
            for v in overriding_vars(*t) {
                cmd.env_remove(v);
            }
        }
    }
    Ok(cmd)
}

/// Run and return the exit code. On Unix the process is replaced (exec), so
/// signals and exit codes behave exactly as if you had run the CLI yourself.
pub fn run(p: &Profile, tool: Tool, args: &[OsString], clean_env: bool) -> Result<i32> {
    warn_overrides(tool, clean_env);
    let mut cmd = command(p, tool, args, clean_env)?;
    p.touch();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        Err(err).with_context(|| format!("starting {}", tool.program()))
    }
    #[cfg(not(unix))]
    {
        // Ctrl+C goes to every process on the console. Let the CLI handle it
        // and keep waiting instead of dying and orphaning it.
        let _ = ctrlc::set_handler(|| {});
        let status = cmd.status().with_context(|| format!("starting {}", tool.program()))?;
        Ok(status.code().unwrap_or(1))
    }
}

/// Run to completion and return the exit code, without exec (used for login).
pub fn run_wait(p: &Profile, tool: Tool, args: &[OsString]) -> Result<i32> {
    warn_overrides(tool, false);
    let mut cmd = command(p, tool, args, false)?;
    #[cfg(not(unix))]
    let _ = ctrlc::set_handler(|| {});
    let status = cmd.status().with_context(|| format!("starting {}", tool.program()))?;
    Ok(status.code().unwrap_or(1))
}

/// Run quietly and capture output (used for `mcp` and the logout before `remove`).
pub fn capture(p: &Profile, tool: Tool, args: &[OsString]) -> Result<Output> {
    command(p, tool, args, false)?.stdin(Stdio::null()).output().with_context(|| format!("starting {}", tool.program()))
}

/// The official login command's arguments; `None` if the tool has no login command.
pub fn login_args(tool: Tool) -> Option<Vec<OsString>> {
    tool.spec().login.map(|a| a.iter().map(OsString::from).collect())
}

/// The official logout command's arguments, if the tool has one.
pub fn logout_args(tool: Tool) -> Option<Vec<OsString>> {
    tool.spec().logout.map(|a| a.iter().map(OsString::from).collect())
}
