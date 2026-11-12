//! `keepapart doctor`: installs, PATH, isolation, links.

use crate::launch::{overriding_vars, program_path};
use crate::paths::same_path;
use crate::profile::{Store, Tool};
use crate::share::{self, is_link, ItemState, CATALOG, NEVER_SHARED};
use anyhow::Result;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Default)]
pub struct Doctor {
    pub errors: usize,
    pub warnings: usize,
}

impl Doctor {
    fn ok(&self, msg: impl AsRef<str>) {
        println!("  ok    {}", msg.as_ref());
    }
    fn info(&self, msg: impl AsRef<str>) {
        println!("  info  {}", msg.as_ref());
    }
    fn warn(&mut self, msg: impl AsRef<str>) {
        self.warnings += 1;
        println!("  warn  {}", msg.as_ref());
    }
    fn fail(&mut self, msg: impl AsRef<str>) {
        self.errors += 1;
        println!("  FAIL  {}", msg.as_ref());
    }
}

/// Can this process create file and folder symlinks here? (Windows needs
/// Developer Mode or admin rights for symlinks; junctions need neither.)
pub fn probe_links(dir: &Path) -> (bool, bool) {
    let probe = dir.join(format!(".keepapart-probe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&probe);
    if fs::create_dir_all(probe.join("d")).is_err() || fs::write(probe.join("f"), "x").is_err() {
        return (false, false);
    }
    #[cfg(unix)]
    let (file_ok, dir_ok) = (
        std::os::unix::fs::symlink(probe.join("f"), probe.join("lf")).is_ok(),
        std::os::unix::fs::symlink(probe.join("d"), probe.join("ld")).is_ok(),
    );
    #[cfg(windows)]
    let (file_ok, dir_ok) = (
        std::os::windows::fs::symlink_file(probe.join("f"), probe.join("lf")).is_ok(),
        std::os::windows::fs::symlink_dir(probe.join("d"), probe.join("ld")).is_ok(),
    );
    for l in ["lf", "ld"] {
        if is_link(&probe.join(l)) {
            let _ = share::remove_link(&probe.join(l));
        }
    }
    let _ = fs::remove_dir_all(&probe);
    (file_ok, dir_ok)
}

pub fn run(store: &Store) -> Result<Doctor> {
    let mut d = Doctor::default();
    println!("keepapart {} ({} {})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);

    println!("\nStore");
    let probe = store.home.join(".write-test");
    match fs::write(&probe, "x") {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            d.ok(format!("{} is writable", store.home.display()));
        }
        Err(e) => d.fail(format!("can't write to {}: {e}", store.home.display())),
    }

    println!("\nOfficial CLIs");
    let profiles = store.list()?;
    for t in Tool::ALL {
        let used = profiles.iter().any(|p| p.has(t));
        match program_path(t) {
            Ok(path) => {
                let version = Command::new(&path)
                    .arg("--version")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "version unknown".into());
                d.ok(format!("{}: {} ({version})", t.program(), path.display()));
            }
            Err(_) if used => d.fail(format!("{} isn't on PATH, but a profile uses it", t.program())),
            Err(_) => d.info(format!("{} isn't on PATH (no profile uses it)", t.program())),
        }
    }

    println!("\nEnvironment");
    for t in Tool::ALL {
        if let Some(v) = std::env::var_os(t.env_var()) {
            if std::env::var_os("KEEPAPART_ACTIVE").is_some() {
                d.ok(format!("{}={} (set by the keepapart shell hook)", t.env_var(), v.to_string_lossy()));
            } else {
                d.info(format!(
                    "{}={} is set in this shell; plain `{}` uses it, `keepapart run` overrides it",
                    t.env_var(),
                    v.to_string_lossy(),
                    t.program()
                ));
            }
        }
        for var in overriding_vars(t) {
            if std::env::var_os(var).is_some_and(|x| !x.is_empty()) {
                d.warn(format!("${var} is set: {} will use it instead of any profile's login", t.program()));
            }
        }
    }

    println!("\nLinks");
    let (file_ok, dir_ok) = probe_links(&store.home);
    if let Ok(m) = std::env::var("KEEPAPART_LINK_MODE") {
        d.info(format!("KEEPAPART_LINK_MODE={m}: new shared items follow it, whatever the OS allows"));
    }
    if file_ok && dir_ok {
        d.ok("symlinks allowed: shared items stay live in every profile");
    } else if cfg!(windows) {
        d.warn(
            "symlinks not allowed for this user. Shared folders use junctions (live); shared files are copied \
             and refreshed by `keepapart share`. To get live shared files, turn on Developer Mode \
             (Settings > System > For developers) and re-run `keepapart share <profile> <item>`.",
        );
    } else {
        d.fail("can't create symlinks in the keepapart folder");
    }

    println!("\nProfiles");
    if profiles.is_empty() {
        d.info("no profiles yet: keepapart add work --tool claude");
    }
    let home = dirs::home_dir();
    let mut seen: Vec<(String, Tool, std::path::PathBuf)> = Vec::new();
    for p in &profiles {
        println!("  [{}] {}", p.name(), p.dir.display());
        for &t in &p.file.tools {
            let dir = p.tool_dir(t);
            if !dir.is_dir() {
                d.fail(format!(
                    "{} folder missing: {} (create it: mkdir it, then keepapart login {})",
                    t.as_str(),
                    dir.display(),
                    p.name()
                ));
                continue;
            }
            if is_link(&dir) {
                d.fail(format!("{} is a link; a profile's {} folder must be its own", dir.display(), t.as_str()));
            }
            if let Some(h) = &home {
                if same_path(&dir, &h.join(t.default_dir_name())) {
                    d.fail(format!("{} is your default ~/{}: not isolated", dir.display(), t.default_dir_name()));
                }
            }
            if let Some((other, _, _)) = seen.iter().find(|(_, tt, pp)| *tt == t && same_path(pp, &dir)) {
                d.fail(format!("{} folder is shared with profile {other}: not isolated", t.as_str()));
            }
            seen.push((p.name().to_string(), t, dir.clone()));

            for (_, name) in NEVER_SHARED.iter().filter(|(nt, _)| *nt == t) {
                let path = dir.join(name);
                if is_link(&path) {
                    d.fail(format!(
                        "{} is a link. Logins and history must stay per profile; delete the link and log in again",
                        path.display()
                    ));
                }
            }
            let login = if p.login_file_present(t) {
                "login file present".to_string()
            } else if t == Tool::Claude && cfg!(target_os = "macos") {
                "login kept in the macOS Keychain, keyed to this folder".to_string()
            } else {
                format!("not logged in yet: keepapart login {}", p.name())
            };
            d.ok(format!("{}: own folder, {login}", t.as_str()));

            for item in CATALOG.iter().filter(|i| i.tool == t) {
                match share::state(store, p, item) {
                    ItemState::Shared(m) => d.ok(format!("{} shared ({})", item.id(), m.as_str())),
                    ItemState::Broken => d.fail(format!(
                        "{} is marked shared but the link is broken: keepapart share {} {}",
                        item.id(),
                        p.name(),
                        item.id()
                    )),
                    ItemState::Private | ItemState::Absent => {}
                }
            }
        }
    }

    println!();
    if d.errors == 0 && d.warnings == 0 {
        println!("All good.");
    } else {
        println!("{} problem(s), {} warning(s).", d.errors, d.warnings);
    }
    Ok(d)
}
