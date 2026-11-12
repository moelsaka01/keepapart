//! End-to-end tests of the keepapart binary with stand-in `claude` / `codex`
//! programs that report the environment they were started with and write a
//! fake login file, the way the real CLIs write theirs.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

struct Env {
    _t: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    work: PathBuf,
}

/// A fake CLI: prints the profile variables and its args, writes them to
/// `$KP_OUT` if set (after an optional delay), and writes a login file in its
/// own config folder, like `claude /login` or `codex login` would.
fn write_fake(dir: &Path, name: &str, var: &str, login_file: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let p = dir.join(format!("{name}.cmd"));
        let body = format!(
            "@echo off\r\n\
             echo {var}=%{var}%\r\n\
             echo ACTIVE=%KEEPAPART_ACTIVE%\r\n\
             echo ARGS=%*\r\n\
             if not \"%KP_DELAY%\"==\"\" ping -n %KP_DELAY% 127.0.0.1 >nul\r\n\
             if not \"%KP_OUT%\"==\"\" echo %{var}%> \"%KP_OUT%\"\r\n\
             echo login-for-%KEEPAPART_ACTIVE%> \"%{var}%\\{login_file}\"\r\n"
        );
        fs::write(&p, body).unwrap();
        p
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        let body = format!(
            "#!/bin/sh\n\
             echo \"{var}=${var}\"\n\
             echo \"ACTIVE=$KEEPAPART_ACTIVE\"\n\
             echo \"ARGS=$*\"\n\
             if [ -n \"$KP_DELAY\" ]; then sleep \"$KP_DELAY\"; fi\n\
             if [ -n \"$KP_OUT\" ]; then printf '%s\\n' \"${var}\" > \"$KP_OUT\"; fi\n\
             echo \"login-for-$KEEPAPART_ACTIVE\" > \"${var}/{login_file}\"\n"
        );
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }
}

fn env() -> Env {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let home = root.join("kp home with spaces");
    let bin = root.join("bin");
    let work = root.join("work");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&work).unwrap();
    write_fake(&bin, "claude", "CLAUDE_CONFIG_DIR", ".credentials.json");
    write_fake(&bin, "codex", "CODEX_HOME", "auth.json");
    Env { _t: t, home, bin, work }
}

impl Env {
    fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("keepapart").unwrap();
        let path = std::env::join_paths(
            std::iter::once(self.bin.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        c.env("KEEPAPART_HOME", &self.home)
            .env("PATH", path)
            .env_remove("KEEPAPART_PROFILE")
            .env_remove("KEEPAPART_ACTIVE")
            .env_remove("KEEPAPART_LINK_MODE")
            .env_remove("KEEPAPART_CLAUDE_BIN")
            .env_remove("KEEPAPART_CODEX_BIN")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("ANTHROPIC_API_KEY")
            .current_dir(&self.work);
        c
    }

    fn profile_dir(&self, id: &str, tool: &str) -> PathBuf {
        self.home.join("profiles").join(id).join(tool)
    }

    fn add(&self, name: &str, tool: &str) {
        self.cmd().args(["add", name, "--tool", tool, "--no-login"]).assert().success();
    }
}

#[test]
fn add_creates_isolated_folders_and_lists_them() {
    let e = env();
    e.add("work", "both");
    e.add("personal", "claude");
    assert!(e.profile_dir("work", "claude").is_dir());
    assert!(e.profile_dir("work", "codex").is_dir());
    assert!(e.profile_dir("personal", "claude").is_dir());
    assert!(!e.profile_dir("personal", "codex").exists());
    e.cmd()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("work").and(predicate::str::contains("claude+codex")))
        .stdout(predicate::str::contains("personal"));
    // adding the same profile again with the same tool is an error...
    e.cmd().args(["add", "work", "--tool", "claude", "--no-login"]).assert().failure();
    // ...but adding a new tool to it works
    e.cmd().args(["add", "personal", "--tool", "codex", "--no-login"]).assert().success();
    assert!(e.profile_dir("personal", "codex").is_dir());
}

#[test]
fn run_sets_only_the_profiles_folder_and_passes_args() {
    let e = env();
    e.add("work", "both");
    let out = e.cmd().args(["run", "work", "--", "--resume", "two words"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    let want = e.profile_dir("work", "claude");
    assert!(s.contains(&format!("CLAUDE_CONFIG_DIR={}", want.display())), "{s}");
    assert!(s.contains("ACTIVE=work"), "{s}");
    assert!(s.contains("--resume"), "{s}");

    let out = e.cmd().args(["run", "work", "--tool", "codex"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains(&format!("CODEX_HOME={}", e.profile_dir("work", "codex").display())), "{s}");
}

#[test]
fn logins_land_in_each_profile_and_never_cross() {
    let e = env();
    e.add("work", "claude");
    e.add("personal", "claude");
    e.cmd().args(["login", "work"]).assert().success();
    e.cmd().args(["login", "personal"]).assert().success();
    let w = fs::read_to_string(e.profile_dir("work", "claude").join(".credentials.json")).unwrap();
    let p = fs::read_to_string(e.profile_dir("personal", "claude").join(".credentials.json")).unwrap();
    assert_eq!(w.trim(), "login-for-work");
    assert_eq!(p.trim(), "login-for-personal");
    assert!(!e.home.join("shared").join("claude").join(".credentials.json").exists());
    e.cmd()
        .args(["share", "work", "claude:.credentials.json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("always kept per profile"));
    e.cmd().args(["share", "work", "codex:auth.json"]).assert().failure();
}

#[test]
fn two_profiles_run_side_by_side() {
    let e = env();
    e.add("work", "claude");
    e.add("personal", "claude");
    let out_a = e.work.join("a.txt");
    let out_b = e.work.join("b.txt");
    let spawn = |name: &str, out: &Path| {
        let mut c = std::process::Command::new(assert_cmd::cargo::cargo_bin("keepapart"));
        let base = e.cmd();
        for (k, v) in base.get_envs() {
            match v {
                Some(v) => c.env(k, v),
                None => c.env_remove(k),
            };
        }
        c.current_dir(&e.work)
            .args(["run", name])
            .env("KP_OUT", out)
            .env("KP_DELAY", "2")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut a = spawn("work", &out_a);
    let mut b = spawn("personal", &out_b);
    assert!(a.wait().unwrap().success());
    assert!(b.wait().unwrap().success());
    let a_dir = fs::read_to_string(&out_a).unwrap();
    let b_dir = fs::read_to_string(&out_b).unwrap();
    assert_eq!(PathBuf::from(a_dir.trim()), e.profile_dir("work", "claude"));
    assert_eq!(PathBuf::from(b_dir.trim()), e.profile_dir("personal", "claude"));
}

#[test]
fn folder_pin_global_default_and_env_override() {
    let e = env();
    e.add("work", "claude");
    e.add("client-acme", "claude");
    e.add("personal", "claude");
    let repo = e.work.join("acme-repo");
    let deep = repo.join("src").join("deep");
    fs::create_dir_all(&deep).unwrap();

    // nothing pinned, no default
    e.cmd().arg("run").assert().failure().stderr(predicate::str::contains("which profile"));

    e.cmd().args(["use", "personal", "--global"]).assert().success();
    e.cmd().args(["use", "client-acme"]).current_dir(&repo).assert().success();
    assert!(fs::read_to_string(repo.join(".keepapart")).unwrap().starts_with("client-acme\n"));

    let who = |dir: &Path, extra: Option<(&str, &str)>| {
        let mut c = e.cmd();
        c.arg("run").current_dir(dir);
        if let Some((k, v)) = extra {
            c.env(k, v);
        }
        let out = c.output().unwrap();
        String::from_utf8_lossy(&out.stdout).lines().find(|l| l.starts_with("ACTIVE=")).unwrap_or("").to_string()
    };
    assert_eq!(who(&deep, None), "ACTIVE=client-acme");
    assert_eq!(who(&e.work, None), "ACTIVE=personal");
    assert_eq!(who(&deep, Some(("KEEPAPART_PROFILE", "work"))), "ACTIVE=work");

    e.cmd()
        .arg("status")
        .current_dir(&deep)
        .assert()
        .success()
        .stdout(predicate::str::contains("client-acme").and(predicate::str::contains(".keepapart")));

    e.cmd().args(["use", "--clear"]).current_dir(&repo).assert().success();
    assert_eq!(who(&deep, None), "ACTIVE=personal");
}

#[test]
fn pin_to_a_missing_profile_is_a_clear_error() {
    let e = env();
    fs::write(e.work.join(".keepapart"), "ghost\n").unwrap();
    e.cmd().arg("run").assert().failure().stderr(predicate::str::contains("ghost"));
}

#[test]
fn rename_keeps_the_login_folder() {
    let e = env();
    e.add("acme", "claude");
    e.cmd().args(["login", "acme"]).assert().success();
    e.cmd().args(["rename", "acme", "acme-corp"]).assert().success();
    let out = e.cmd().args(["run", "acme-corp"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains(&format!("CLAUDE_CONFIG_DIR={}", e.profile_dir("acme", "claude").display())), "{s}");
    e.cmd().args(["run", "acme"]).assert().failure();
}

#[test]
fn remove_needs_confirmation_and_spares_shared_content() {
    let e = env();
    e.add("work", "claude");
    e.add("personal", "claude");
    fs::write(e.profile_dir("work", "claude").join("skills").join("note.md"), "shared skill").unwrap();
    e.cmd().args(["remove", "work"]).assert().failure().stderr(predicate::str::contains("--yes"));
    e.cmd().args(["remove", "work", "--yes"]).assert().success();
    assert!(!e.home.join("profiles").join("work").exists());
    assert_eq!(
        fs::read_to_string(e.profile_dir("personal", "claude").join("skills").join("note.md")).unwrap(),
        "shared skill"
    );
}

#[test]
fn copy_mode_works_where_links_are_not_allowed() {
    let e = env();
    let mut c = e.cmd();
    c.env("KEEPAPART_LINK_MODE", "copy").args(["add", "w", "--tool", "claude", "--no-login"]).assert().success();
    let toml = fs::read_to_string(e.home.join("profiles").join("w").join("profile.toml")).unwrap();
    assert!(toml.contains("mode = \"copy\""), "{toml}");
    assert!(!keepapart::share::is_link(&e.profile_dir("w", "claude").join("skills")));
    e.cmd().arg("doctor").env("KEEPAPART_LINK_MODE", "copy").assert().stdout(predicate::str::contains("copy"));
}

#[test]
fn export_has_settings_but_no_logins() {
    let e = env();
    e.add("work", "both");
    e.cmd().args(["login", "work"]).assert().success();
    let claude = e.profile_dir("work", "claude");
    fs::write(claude.join("settings.json"), r#"{"model":"opus","env":{"ANTHROPIC_API_KEY":"sk-ant-secret"}}"#).unwrap();
    fs::write(
        e.profile_dir("work", "codex").join("config.toml"),
        "model = \"gpt-5\"\n[mcp_servers.gh]\ncommand = \"npx\"\nenv = { GITHUB_TOKEN = \"ghp_secret\" }\n",
    )
    .unwrap();
    let out = e.work.join("exp");
    e.cmd().args(["export-config", "work", "--out"]).arg(&out).assert().success();

    let mut all = String::new();
    let mut names = vec![];
    fn walk(p: &Path, all: &mut String, names: &mut Vec<String>) {
        for e in fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            if e.path().is_dir() {
                walk(&e.path(), all, names);
            } else {
                names.push(e.file_name().to_string_lossy().into_owned());
                all.push_str(&fs::read_to_string(e.path()).unwrap_or_default());
            }
        }
    }
    walk(&out, &mut all, &mut names);
    assert!(names.contains(&"settings.json".to_string()));
    assert!(names.contains(&"config.toml".to_string()));
    assert!(!names.iter().any(|n| n == ".credentials.json" || n == "auth.json"), "{names:?}");
    assert!(all.contains("opus") && all.contains("gpt-5"));
    assert!(!all.contains("sk-ant-secret") && !all.contains("ghp_secret") && !all.contains("login-for-work"));
}

#[test]
fn shell_env_follows_the_folder() {
    let e = env();
    e.add("work", "both");
    e.cmd().args(["use", "work"]).assert().success();
    e.cmd()
        .args(["env", "--shell", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("export CLAUDE_CONFIG_DIR='"))
        .stdout(predicate::str::contains("export CODEX_HOME='"))
        .stdout(predicate::str::contains("export KEEPAPART_ACTIVE='work'"));
    e.cmd()
        .args(["env", "--shell", "powershell"])
        .assert()
        .stdout(predicate::str::contains("$env:CLAUDE_CONFIG_DIR = '"));
    // leaving the folder: only unset what keepapart set
    let elsewhere = e.home.parent().unwrap().to_path_buf();
    e.cmd()
        .args(["env", "--shell", "fish"])
        .current_dir(&elsewhere)
        .env("KEEPAPART_ACTIVE", "work")
        .env("CLAUDE_CONFIG_DIR", "x")
        .assert()
        .stdout(predicate::str::contains("set -e CLAUDE_CONFIG_DIR"));
    for sh in ["bash", "zsh", "fish", "powershell"] {
        e.cmd().args(["hook", sh]).assert().success().stdout(predicate::str::contains("keepapart env --shell"));
    }
}

#[test]
fn finds_the_cli_on_path_and_doctor_is_happy() {
    let e = env();
    e.add("work", "both");
    e.cmd()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("claude:").and(predicate::str::contains("own folder")));
    // a missing CLI is reported clearly by run
    e.cmd()
        .args(["run", "work"])
        .env("PATH", "")
        .assert()
        .failure()
        .stderr(predicate::str::contains("isn't on your PATH"));
}

#[test]
fn api_key_in_env_is_flagged() {
    let e = env();
    e.add("work", "claude");
    e.cmd()
        .args(["run", "work"])
        .env("ANTHROPIC_API_KEY", "x")
        .assert()
        .success()
        .stderr(predicate::str::contains("ANTHROPIC_API_KEY"));
}

#[test]
fn shared_listing_mentions_mcp_limits() {
    let e = env();
    e.add("work", "claude");
    e.cmd()
        .arg("shared")
        .assert()
        .success()
        .stdout(predicate::str::contains("claude:skills").and(predicate::str::contains("work:shared")))
        .stdout(predicate::str::contains("MCP"));
}

#[test]
fn without_symlink_rights_folders_still_share_live_and_removal_is_safe() {
    let e = env();
    for n in ["a", "b"] {
        e.cmd()
            .env("KEEPAPART_LINK_MODE", "no-symlink")
            .args(["add", n, "--tool", "claude", "--no-login"])
            .assert()
            .success();
    }
    let toml = fs::read_to_string(e.home.join("profiles").join("a").join("profile.toml")).unwrap();
    if cfg!(windows) {
        // folders: junctions (no admin or Developer Mode needed); files: copies
        assert!(toml.contains("item = \"claude:skills\"\nmode = \"junction\""), "{toml}");
        assert!(toml.contains("item = \"claude:CLAUDE.md\"\nmode = \"copy\""), "{toml}");
        fs::write(e.profile_dir("a", "claude").join("skills").join("s.md"), "live").unwrap();
        assert_eq!(fs::read_to_string(e.profile_dir("b", "claude").join("skills").join("s.md")).unwrap(), "live");
    } else {
        assert!(toml.contains("mode = \"copy\""), "{toml}");
        fs::write(e.home.join("shared").join("claude").join("skills").join("s.md"), "live").unwrap();
    }
    e.cmd().args(["remove", "a", "--yes"]).assert().success();
    assert_eq!(fs::read_to_string(e.home.join("shared").join("claude").join("skills").join("s.md")).unwrap(), "live");
    let doc = e.cmd().arg("doctor").assert().success();
    if cfg!(windows) {
        doc.stdout(predicate::str::contains("claude:skills shared (junction)"));
    }
}
