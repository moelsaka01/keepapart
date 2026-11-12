//! Launching the official CLIs with a profile's environment.
//!
//! keepapart sets exactly two documented variables, CLAUDE_CONFIG_DIR and
//! CODEX_HOME, plus KEEPAPART_ACTIVE (informational), and then runs the real
//! `claude` / `codex`. Each launch is its own process with its own
//! environment, so any number of profiles can run side by side.

use crate::profile::{Profile, Tool};
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

/// Where the official CLI lives: `$KEEPAPART_CLAUDE_BIN` / `$KEEPAPART_CODEX_BIN`,
/// else PATH (with PATHEXT on Windows, so `claude.cmd` from npm works too).
pub fn program_path(tool: Tool) -> Result<PathBuf> {
    let var = bin_override_var(tool);
    if let Some(p) = std::env::var_os(var).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    which::which(tool.program()).map_err(|_| anyhow::anyhow!(not_found(tool)))
}

fn not_found(tool: Tool) -> String {
    let var = bin_override_var(tool);
    if let Some(found) = installed_off_path(tool) {
        return format!(
            "`{}` is installed at {} but that folder isn't on your PATH. Add {} to your PATH{}, or set {var} to the full path",
            tool.program(),
            found.display(),
            found.parent().map(|p| p.display().to_string()).unwrap_or_default(),
            if cfg!(windows) { " (Start > \"Edit environment variables for your account\" > Path)" } else { "" },
        );
    }
    let how = match tool {
        Tool::Claude => "https://code.claude.com/docs/en/setup",
        Tool::Codex => "https://developers.openai.com/codex/cli",
    };
    format!("`{}` isn't on your PATH. Install it first: {how}", tool.program())
}

pub fn bin_override_var(tool: Tool) -> &'static str {
    match tool {
        Tool::Claude => "KEEPAPART_CLAUDE_BIN",
        Tool::Codex => "KEEPAPART_CODEX_BIN",
    }
}

/// The official native installers put the CLI in `~/.local/bin`, which often
/// isn't on PATH yet (the Claude Code installer says so on Windows).
pub fn installed_off_path(tool: Tool) -> Option<PathBuf> {
    let dir = dirs::home_dir()?.join(".local").join("bin");
    let name = if cfg!(windows) { format!("{}.exe", tool.program()) } else { tool.program().to_string() };
    let p = dir.join(name);
    p.is_file().then_some(p)
}

/// The environment a profile adds. Both tool variables are set when the
/// profile has both tools, so a `claude` session that shells out to `codex`
/// stays on the same profile.
pub fn profile_env(p: &Profile) -> Vec<(&'static str, OsString)> {
    let mut env: Vec<(&'static str, OsString)> =
        p.file.tools.iter().map(|t| (t.env_var(), p.tool_dir(*t).into_os_string())).collect();
    env.push(("KEEPAPART_ACTIVE", p.name().into()));
    env
}

/// Variables that make the official CLI ignore the profile's login.
pub fn overriding_vars(tool: Tool) -> &'static [&'static str] {
    match tool {
        Tool::Claude => &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"],
        Tool::Codex => &[],
    }
}

pub fn warn_overrides(tool: Tool) {
    for v in overriding_vars(tool) {
        if std::env::var_os(v).is_some_and(|x| !x.is_empty()) {
            eprintln!(
                "keepapart: warning: ${v} is set in your environment; {} will use it instead of this profile's login",
                tool.as_str()
            );
        }
    }
}

pub fn command(p: &Profile, tool: Tool, args: &[OsString]) -> Result<Command> {
    let prog = program_path(tool)?;
    let mut cmd = Command::new(prog);
    cmd.args(args);
    for (k, v) in profile_env(p) {
        cmd.env(k, v);
    }
    // A profile that doesn't use the other tool shouldn't inherit a stale value
    // from a shell hook either.
    for t in Tool::ALL {
        if !p.has(t) && std::env::var_os("KEEPAPART_ACTIVE").is_some() {
            cmd.env_remove(t.env_var());
        }
    }
    Ok(cmd)
}

/// Run and return the exit code. On Unix the process is replaced (exec), so
/// signals and exit codes behave exactly as if you had run the CLI yourself.
pub fn run(p: &Profile, tool: Tool, args: &[OsString]) -> Result<i32> {
    warn_overrides(tool);
    let mut cmd = command(p, tool, args)?;
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
    warn_overrides(tool);
    let mut cmd = command(p, tool, args)?;
    #[cfg(not(unix))]
    let _ = ctrlc::set_handler(|| {});
    let status = cmd.status().with_context(|| format!("starting {}", tool.program()))?;
    Ok(status.code().unwrap_or(1))
}

/// The official login commands: `claude auth login` and `codex login`.
pub fn login_args(tool: Tool) -> Vec<OsString> {
    match tool {
        Tool::Claude => vec!["auth".into(), "login".into()],
        Tool::Codex => vec!["login".into()],
    }
}
