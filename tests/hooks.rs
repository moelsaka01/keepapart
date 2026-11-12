//! The shell hooks, run in real shells. Each test is skipped when its shell
//! isn't installed. CI installs bash, zsh and fish on Linux and macOS, and
//! runs PowerShell on Windows. POSIX shells aren't tried on Windows, where
//! `bash` may be WSL's.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Setup {
    _t: tempfile::TempDir,
    home: PathBuf,
    pinned: PathBuf,
    elsewhere: PathBuf,
    path: std::ffi::OsString,
}

fn setup() -> Setup {
    let t = tempfile::tempdir().unwrap();
    // canonicalize() gives \\?\C:\... on Windows; shells want the plain form.
    let canon = t.path().canonicalize().unwrap().display().to_string();
    let root = PathBuf::from(canon.strip_prefix(r"\\?\").unwrap_or(&canon));
    let home = root.join("kp");
    let pinned = root.join("client repo");
    let elsewhere = root.join("other");
    fs::create_dir_all(&pinned).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    let bin = assert_cmd::cargo::cargo_bin("keepapart");
    let dir = bin.parent().unwrap().to_path_buf();
    let path =
        std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())))
            .unwrap();
    let s = Setup { _t: t, home, pinned, elsewhere, path };
    for args in [
        vec!["add", "client", "--tool", "both", "--no-login"],
        vec!["use", "client"],
        vec!["hook", "powershell", "--write"],
    ] {
        let st = s.cmd(&bin).args(&args).current_dir(&s.pinned).status().unwrap();
        assert!(st.success());
    }
    s
}

impl Setup {
    fn cmd(&self, prog: &Path) -> Command {
        let mut c = Command::new(prog);
        c.env("KEEPAPART_HOME", &self.home)
            .env("PATH", &self.path)
            .env("KP_PINNED", &self.pinned)
            .env("KP_ELSEWHERE", &self.elsewhere)
            .env("KP_HOOK", self.home.join("hook.ps1"))
            .env_remove("KEEPAPART_PROFILE")
            .env_remove("KEEPAPART_ACTIVE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME");
        c
    }

    fn want(&self, tool: &str) -> String {
        self.home.join("profiles").join("client").join(tool).display().to_string()
    }

    /// Run `script` in `shell`; it prints CLAUDE_CONFIG_DIR|CODEX_HOME|KEEPAPART_ACTIVE
    /// inside the pinned folder, then the same after moving elsewhere.
    fn check(&self, shell: &str, args: &[&str], script: &str) {
        let Ok(prog) = which::which(shell) else {
            eprintln!("skipping: {shell} not installed");
            return;
        };
        let out = self.cmd(&prog).args(args).arg(script).output().unwrap();
        let so = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = so.lines().map(str::trim).filter(|l| l.starts_with("R:")).collect();
        assert_eq!(lines.len(), 2, "{shell}: {so}\n{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(lines[0], format!("R:{}|{}|client", self.want("claude"), self.want("codex")), "{shell}");
        assert_eq!(lines[1], "R:||", "{shell} should unset what it set");
    }
}

#[cfg(unix)]
#[test]
fn bash_hook() {
    let s = setup();
    s.check(
        "bash",
        &["--norc", "-c"],
        r#"eval "$(keepapart hook bash)"; cd "$KP_PINNED"; _keepapart_hook; echo "R:$CLAUDE_CONFIG_DIR|$CODEX_HOME|$KEEPAPART_ACTIVE"; cd "$KP_ELSEWHERE"; _keepapart_hook; echo "R:$CLAUDE_CONFIG_DIR|$CODEX_HOME|$KEEPAPART_ACTIVE""#,
    );
}

#[cfg(unix)]
#[test]
fn zsh_hook() {
    let s = setup();
    s.check(
        "zsh",
        &["-f", "-c"],
        r#"eval "$(keepapart hook zsh)"; cd "$KP_PINNED"; _keepapart_hook; echo "R:$CLAUDE_CONFIG_DIR|$CODEX_HOME|$KEEPAPART_ACTIVE"; cd "$KP_ELSEWHERE"; _keepapart_hook; echo "R:$CLAUDE_CONFIG_DIR|$CODEX_HOME|$KEEPAPART_ACTIVE""#,
    );
}

#[cfg(unix)]
#[test]
fn fish_hook() {
    let s = setup();
    s.check(
        "fish",
        &["--no-config", "-c"],
        r#"keepapart hook fish | source; cd "$KP_PINNED"; __keepapart_hook; echo "R:$CLAUDE_CONFIG_DIR|$CODEX_HOME|$KEEPAPART_ACTIVE"; cd "$KP_ELSEWHERE"; __keepapart_hook; echo "R:$CLAUDE_CONFIG_DIR|$CODEX_HOME|$KEEPAPART_ACTIVE""#,
    );
}

// Dot-sources the file written by `keepapart hook powershell --write`; no Invoke-Expression anywhere.
const PS: &str = r#". $env:KP_HOOK; Set-Location -LiteralPath $env:KP_PINNED; prompt | Out-Null; "R:$($env:CLAUDE_CONFIG_DIR)|$($env:CODEX_HOME)|$($env:KEEPAPART_ACTIVE)"; Set-Location -LiteralPath $env:KP_ELSEWHERE; prompt | Out-Null; "R:$($env:CLAUDE_CONFIG_DIR)|$($env:CODEX_HOME)|$($env:KEEPAPART_ACTIVE)""#;

#[cfg(windows)]
#[test]
fn windows_powershell_hook() {
    let s = setup();
    s.check("powershell", &["-NoProfile", "-NonInteractive", "-Command"], PS);
}

#[test]
fn pwsh_hook() {
    let s = setup();
    s.check("pwsh", &["-NoProfile", "-NonInteractive", "-Command"], PS);
}
