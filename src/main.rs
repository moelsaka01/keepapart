use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use keepapart::paths::{ago, now_secs};
use keepapart::profile::{Profile, Store, Tool, ToolChoice};
use keepapart::resolve::{self, Resolved, Source};
use keepapart::share::{self, ItemState, ShareOutcome, CATALOG};
use keepapart::shell::{self, Desired, Shell};
use keepapart::{doctor, export, launch};
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "keepapart",
    version,
    about = "One machine, many Claude & Codex accounts. Work, personal, clients, kept apart.",
    long_about = "One machine, many Claude & Codex accounts. Work, personal, clients, kept apart.\n\n\
        Each profile is its own CLAUDE_CONFIG_DIR / CODEX_HOME. You log in with the official CLI; \
        keepapart never reads, copies or moves your credentials and makes no network calls.\n\n\
        Run with no arguments to pick a profile and launch it.",
    after_help = "Examples:\n  keepapart add work --tool both\n  keepapart run work\n  keepapart run personal -- --resume\n  keepapart use client-acme        # this folder always uses client-acme\n  keepapart doctor"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a profile (or add a tool to one) and log in with the official CLI
    Add {
        name: String,
        /// Which CLI this profile is for
        #[arg(long, short, value_enum, default_value = "claude")]
        tool: ToolChoice,
        /// Share nothing with other profiles (default: skills, agents, commands, instructions are shared)
        #[arg(long)]
        isolated: bool,
        /// Skip the login step
        #[arg(long)]
        no_login: bool,
    },
    /// Log a profile in (or again) with the official CLI
    Login {
        name: String,
        #[arg(long, short, value_enum)]
        tool: Option<Tool>,
    },
    /// Launch claude or codex with a profile. Several can run at once.
    #[command(alias = "r")]
    Run {
        /// Profile (default: the one for this folder, see `keepapart status`)
        name: Option<String>,
        /// Which CLI, for profiles that have both (default: claude)
        #[arg(long, short, value_enum)]
        tool: Option<Tool>,
        /// Arguments passed to claude/codex, after --
        #[arg(last = true)]
        args: Vec<OsString>,
    },
    /// Make a profile the default for this folder (writes .keepapart) or globally
    Use {
        name: Option<String>,
        /// Set the global default instead of this folder's
        #[arg(long, short)]
        global: bool,
        /// Folder to pin (default: current folder)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Remove the folder's .keepapart file (or the global default with --global)
        #[arg(long, conflicts_with = "name")]
        clear: bool,
    },
    /// List profiles
    #[command(alias = "ls")]
    List,
    /// Show which profile applies here and why
    Status,
    /// Share items (skills, agents, ...) between this profile and others
    Share {
        profile: String,
        /// Items like claude:skills codex:prompts (see `keepapart shared`)
        #[arg(required = true)]
        items: Vec<String>,
        /// If both sides have the item, use the shared one and back up this profile's copy
        #[arg(long)]
        force: bool,
    },
    /// Keep items per profile again (the profile gets its own copy)
    Unshare {
        profile: String,
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Show what can be shared and what each profile shares
    Shared { profile: Option<String> },
    /// Delete a profile, its login files and history included
    #[command(alias = "rm")]
    Remove {
        name: String,
        #[arg(long, short)]
        yes: bool,
    },
    /// Rename a profile (its folders stay put, so the login survives)
    #[command(alias = "mv")]
    Rename { old: String, new: String },
    /// Export a profile's settings (never logins) to a folder
    ExportConfig {
        name: String,
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Check installs, PATH, isolation and link permissions
    Doctor,
    /// Print a shell hook that applies the folder's profile to plain claude/codex
    Hook {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Print env commands for the current folder's profile (used by the hook)
    Env {
        #[arg(long, value_enum)]
        shell: Shell,
    },
}

fn main() {
    let code = match real_main() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("keepapart: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

fn real_main() -> Result<i32> {
    let cli = Cli::parse();
    let store = Store::open()?;
    let cwd = std::env::current_dir().context("reading the current folder")?;
    match cli.cmd {
        None => pick(&store, &cwd),
        Some(Cmd::Add { name, tool, isolated, no_login }) => add(&store, &name, tool, isolated, no_login),
        Some(Cmd::Login { name, tool }) => {
            let p = store.find(&name)?;
            let tools = match tool {
                Some(t) => vec![t],
                None => p.file.tools.clone(),
            };
            for t in tools {
                if !p.has(t) {
                    bail!(
                        "profile {name:?} doesn't use {}; add it: keepapart add {name} --tool {}",
                        t.as_str(),
                        t.as_str()
                    );
                }
                login(&p, t)?;
            }
            Ok(0)
        }
        Some(Cmd::Run { name, tool, args }) => {
            let r = need_profile(&store, name.as_deref(), &cwd)?;
            let p = store.find(&r.name).with_context(|| format!("profile {}", r.source.describe()))?;
            let t = pick_tool(&p, tool)?;
            if r.source != Source::Explicit {
                eprintln!("keepapart: {} as {:?} ({})", t.as_str(), p.name(), r.source.describe());
            }
            launch::run(&p, t, &args)
        }
        Some(Cmd::Use { name, global, dir, clear }) => use_cmd(&store, name, global, dir, clear, &cwd),
        Some(Cmd::List) => list(&store, &cwd),
        Some(Cmd::Status) => status(&store, &cwd),
        Some(Cmd::Share { profile, items, force }) => {
            let mut p = store.find(&profile)?;
            for id in items {
                let item = share::lookup(&id)?;
                match share::share(&store, &mut p, item, force)? {
                    ShareOutcome::Linked(m) => println!("{id}: shared ({})", m.as_str()),
                    ShareOutcome::AlreadyShared(m) => println!("{id}: already shared ({})", m.as_str()),
                    ShareOutcome::Seeded(m) => {
                        println!("{id}: shared ({}); this profile's copy is now the shared one", m.as_str())
                    }
                    ShareOutcome::BackedUp(m, b) => {
                        println!("{id}: shared ({}); this profile's old copy is at {}", m.as_str(), b.display())
                    }
                }
            }
            Ok(0)
        }
        Some(Cmd::Unshare { profile, items }) => {
            let mut p = store.find(&profile)?;
            for id in items {
                let item = share::lookup(&id)?;
                share::unshare(&store, &mut p, item)?;
                println!("{id}: now per profile ({} has its own copy)", p.name());
            }
            Ok(0)
        }
        Some(Cmd::Shared { profile }) => shared(&store, profile.as_deref()),
        Some(Cmd::Remove { name, yes }) => remove(&store, &name, yes),
        Some(Cmd::Rename { old, new }) => {
            store.rename(&old, &new)?;
            println!("Renamed {old} -> {new}. Its folders didn't move, so its logins still work.");
            println!("Folders pinned with a .keepapart file that says {old:?} need `keepapart use {new}` there.");
            Ok(0)
        }
        Some(Cmd::ExportConfig { name, out }) => {
            let p = store.find(&name)?;
            let out = out.unwrap_or_else(|| cwd.join(format!("keepapart-export-{name}")));
            let r = export::export(&p, &out)?;
            println!("Exported {} item(s) to {}", r.exported.len(), out.display());
            for i in &r.exported {
                println!("  {i}");
            }
            for (i, why) in &r.skipped {
                println!("  skipped {i}: {why}");
            }
            if r.redactions > 0 {
                println!("{} secret-looking value(s) replaced with \"{}\".", r.redactions, export::REDACTED);
            }
            println!("No logins, tokens or session history are included.");
            Ok(0)
        }
        Some(Cmd::Doctor) => {
            let d = doctor::run(&store)?;
            Ok(if d.errors > 0 { 1 } else { 0 })
        }
        Some(Cmd::Hook { shell }) => {
            print!("{}", shell::hook(shell));
            if std::io::stdout().is_terminal() {
                eprintln!("\n# {}", shell::install_hint(shell));
            }
            Ok(0)
        }
        Some(Cmd::Env { shell }) => {
            let desired = match resolve::resolve(&store, None, &cwd) {
                Ok(Some(r)) => match store.try_find(&r.name)? {
                    Some(p) => Some(Desired {
                        profile: p.name().to_string(),
                        dirs: p.file.tools.iter().map(|t| (*t, p.tool_dir(*t).display().to_string())).collect(),
                    }),
                    None => {
                        eprintln!("keepapart: no profile named {:?} ({})", r.name, r.source.describe());
                        None
                    }
                },
                Ok(None) => None,
                Err(e) => {
                    eprintln!("keepapart: {e:#}");
                    None
                }
            };
            let cur = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
            print!("{}", shell::env_script(shell, desired.as_ref(), &cur));
            Ok(0)
        }
    }
}

fn need_profile(store: &Store, name: Option<&str>, cwd: &std::path::Path) -> Result<Resolved> {
    resolve::resolve(store, name, cwd)?.context(
        "which profile? Name one (keepapart run work), pin one to this folder (keepapart use work) or set a default (keepapart use work --global)",
    )
}

fn pick_tool(p: &Profile, tool: Option<Tool>) -> Result<Tool> {
    match tool {
        Some(t) if p.has(t) => Ok(t),
        Some(t) => bail!(
            "profile {:?} doesn't use {}; add it: keepapart add {} --tool {}",
            p.name(),
            t.as_str(),
            p.name(),
            t.as_str()
        ),
        None => Ok(p.file.tools[0]),
    }
}

fn login(p: &Profile, t: Tool) -> Result<()> {
    println!();
    match t {
        Tool::Claude => {
            println!("Starting Claude Code as profile {:?}.", p.name());
            println!("Log in with the account for this profile (it asks on first start; otherwise type /login),");
            println!("then type /exit to come back here.");
        }
        Tool::Codex => {
            println!("Running `codex login` as profile {:?}. Sign in with the account for this profile.", p.name());
        }
    }
    println!("Login is handled entirely by the official CLI. keepapart never sees it.\n");
    let code = launch::run_wait(p, t, &launch::login_args(t))?;
    if code != 0 {
        eprintln!("keepapart: {} exited with code {code}; run `keepapart login {}` to try again", t.as_str(), p.name());
    }
    Ok(())
}

fn add(store: &Store, name: &str, choice: ToolChoice, isolated: bool, no_login: bool) -> Result<i32> {
    let tools = choice.tools();
    let (mut p, new_tools) = match store.try_find(name)? {
        Some(mut p) => {
            let mut added = vec![];
            for t in &tools {
                if store.add_tool(&mut p, *t)? {
                    added.push(*t);
                }
            }
            if added.is_empty() {
                bail!("profile {name:?} already exists with {}", p.tools_label());
            }
            println!(
                "Added {} to profile {name:?}.",
                added.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(" and ")
            );
            (p, added)
        }
        None => {
            let p = store.create(name, &tools)?;
            println!("Created profile {name:?} ({}) at {}", p.tools_label(), p.dir.display());
            (p, tools.clone())
        }
    };
    if !isolated {
        let outcomes = share::apply_defaults(store, &mut p)?;
        let ok: Vec<String> = outcomes
            .iter()
            .filter(|(id, r)| r.is_ok() && new_tools.iter().any(|t| id.starts_with(t.as_str())))
            .map(|(id, _)| id.clone())
            .collect();
        if !ok.is_empty() {
            println!("Shared with your other profiles: {}", ok.join(", "));
        }
        for (id, r) in &outcomes {
            if let Err(e) = r {
                eprintln!("keepapart: couldn't share {id}: {e:#}");
            }
        }
    }
    println!("Kept per profile, always: login, session history, projects.");
    if no_login {
        println!("\nLog in later with: keepapart login {name}");
        return Ok(0);
    }
    for t in new_tools {
        if launch::program_path(t).is_err() {
            eprintln!(
                "keepapart: `{}` isn't installed or on PATH; install it, then run: keepapart login {name}",
                t.as_str()
            );
            continue;
        }
        login(&p, t)?;
    }
    println!("\nDone. Start it with: keepapart run {name}");
    Ok(0)
}

fn use_cmd(
    store: &Store,
    name: Option<String>,
    global: bool,
    dir: Option<PathBuf>,
    clear: bool,
    cwd: &std::path::Path,
) -> Result<i32> {
    let dir = dir.unwrap_or_else(|| cwd.to_path_buf());
    if clear {
        if global {
            let mut cfg = store.config()?;
            cfg.default = None;
            store.save_config(&cfg)?;
            println!("Cleared the global default profile.");
        } else {
            let m = dir.join(keepapart::paths::MARKER_FILE);
            if m.is_file() {
                std::fs::remove_file(&m)?;
                println!("Removed {}", m.display());
            } else {
                println!("No .keepapart file in {}", dir.display());
            }
        }
        return Ok(0);
    }
    let name = name.context("which profile? keepapart use <name>")?;
    let p = store.find(&name)?;
    if global {
        let mut cfg = store.config()?;
        cfg.default = Some(p.name().to_string());
        store.save_config(&cfg)?;
        println!("{name:?} is now the default everywhere without a .keepapart file.");
    } else {
        let path = resolve::write_marker(&dir, p.name())?;
        println!("Wrote {}. `keepapart run` in this folder (and below) uses {name:?}.", path.display());
        if std::env::var_os("KEEPAPART_ACTIVE").is_none() {
            println!("Want plain `claude` / `codex` to follow it too? See `keepapart hook --help`.");
        }
    }
    Ok(0)
}

fn login_label(p: &Profile) -> String {
    p.file
        .tools
        .iter()
        .map(|t| {
            let s = if p.login_file_present(*t) {
                "yes"
            } else if *t == Tool::Claude && cfg!(target_os = "macos") {
                "keychain"
            } else {
                "no"
            };
            format!("{}:{s}", t.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn list(store: &Store, cwd: &std::path::Path) -> Result<i32> {
    let profiles = store.list()?;
    if profiles.is_empty() {
        println!("No profiles yet. Create one: keepapart add work --tool claude");
        return Ok(0);
    }
    let here = resolve::resolve(store, None, cwd).ok().flatten().map(|r| r.name);
    let default = store.config()?.default;
    let names: Vec<String> = profiles
        .iter()
        .map(|p| {
            if default.as_deref() == Some(p.name()) {
                format!("{} (default)", p.name())
            } else {
                p.name().to_string()
            }
        })
        .collect();
    let w = names.iter().map(|n| n.len()).max().unwrap_or(4).max(4);
    println!("  {:w$}  {:12}  {:24}  LAST USED", "NAME", "TOOLS", "LOGIN FILE");
    let now = now_secs();
    for (p, name) in profiles.iter().zip(&names) {
        let mark = if here.as_deref() == Some(p.name()) { "*" } else { " " };
        println!("{mark} {name:w$}  {:12}  {:24}  {}", p.tools_label(), login_label(p), ago(p.last_used(), now));
    }
    if here.is_some() {
        println!("\n* active in this folder");
    }
    Ok(0)
}

fn status(store: &Store, cwd: &std::path::Path) -> Result<i32> {
    match resolve::resolve(store, None, cwd)? {
        None => {
            println!(
                "No profile applies here. Plain `claude` / `codex` use their normal folders (~/.claude, ~/.codex)."
            );
            println!("Pin one: keepapart use <name>   or set a default: keepapart use <name> --global");
        }
        Some(r) => {
            let Some(p) = store.try_find(&r.name)? else {
                bail!(
                    "profile {:?} ({}) doesn't exist; create it: keepapart add {}",
                    r.name,
                    r.source.describe(),
                    r.name
                );
            };
            println!("Profile: {} ({})", p.name(), r.source.describe());
            for t in &p.file.tools {
                println!("  {:6}  {}={}", t.as_str(), t.env_var(), p.tool_dir(*t).display());
            }
            println!("  login   {}", login_label(&p));
            println!("  used    {}", ago(p.last_used(), now_secs()));
            let shared: Vec<&str> = p.file.shared.iter().map(|e| e.item.as_str()).collect();
            println!("  shared  {}", if shared.is_empty() { "nothing".into() } else { shared.join(", ") });
            match std::env::var("KEEPAPART_ACTIVE") {
                Ok(a) if a == p.name() => println!("  shell   hook active: plain claude/codex use this profile"),
                Ok(a) => println!("  shell   hook says {a:?}; it updates at your next prompt"),
                Err(_) => println!("  shell   no hook: use `keepapart run`, or see `keepapart hook --help`"),
            }
        }
    }
    println!("\nPlan usage and limits: run /usage inside Claude Code, or /status inside Codex.");
    println!("keepapart doesn't fetch them: that would mean reading your login or calling unofficial endpoints.");
    Ok(0)
}

fn shared(store: &Store, profile: Option<&str>) -> Result<i32> {
    let profiles = match profile {
        Some(n) => vec![store.find(n)?],
        None => store.list()?,
    };
    println!("Shareable items (logins and session history never are):");
    for item in CATALOG {
        let users: Vec<String> = profiles
            .iter()
            .filter(|p| p.has(item.tool))
            .map(|p| match share::state(store, p, item) {
                ItemState::Shared(m) => format!("{}:shared/{}", p.name(), m.as_str()),
                ItemState::Broken => format!("{}:BROKEN", p.name()),
                ItemState::Private => format!("{}:own", p.name()),
                ItemState::Absent => format!("{}:-", p.name()),
            })
            .collect();
        println!(
            "  {:22} {:52} {}",
            item.id(),
            format!("{}{}", item.about, if item.default_shared { " [default]" } else { "" }),
            users.join("  ")
        );
    }
    println!(
        "\nClaude Code keeps user-level MCP servers in .claude.json next to account data, so those stay per profile."
    );
    println!("Share them through a plugin (claude:plugins) or a project .mcp.json instead.");
    Ok(0)
}

fn remove(store: &Store, name: &str, yes: bool) -> Result<i32> {
    let p = store.find(name)?;
    println!("This deletes {}", p.dir.display());
    println!("including profile {name:?}'s login files and session history. Shared items are untouched.");
    if p.has(Tool::Claude) && cfg!(target_os = "macos") {
        println!("Claude Code's Keychain entry for this folder stays; keepapart never touches the Keychain.");
    }
    println!(
        "To sign out on the server first: keepapart run {name} (then /logout){}",
        if p.has(Tool::Codex) { format!(", and keepapart run {name} --tool codex -- logout") } else { String::new() }
    );
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("not deleting without confirmation; pass --yes");
        }
        let ok =
            dialoguer::Confirm::new().with_prompt(format!("Delete profile {name:?}?")).default(false).interact()?;
        if !ok {
            println!("Kept.");
            return Ok(1);
        }
    }
    store.remove(name)?;
    println!("Deleted profile {name:?}.");
    Ok(0)
}

fn pick(store: &Store, cwd: &std::path::Path) -> Result<i32> {
    let profiles = store.list()?;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("no command given. See `keepapart --help`");
    }
    if profiles.is_empty() {
        println!(
            "keepapart: no profiles yet.\n\n  keepapart add work --tool claude\n  keepapart add personal --tool both\n"
        );
        println!("See `keepapart --help` for everything else.");
        return Ok(0);
    }
    let here = resolve::resolve(store, None, cwd).ok().flatten().map(|r| r.name);
    let now = now_secs();
    let w = profiles.iter().map(|p| p.name().len()).max().unwrap_or(4);
    let labels: Vec<String> = profiles
        .iter()
        .map(|p| {
            format!(
                "{:w$}  {:12}  {:10}{}",
                p.name(),
                p.tools_label(),
                ago(p.last_used(), now),
                if here.as_deref() == Some(p.name()) { "  (this folder)" } else { "" }
            )
        })
        .collect();
    let default = here.and_then(|h| profiles.iter().position(|p| p.name() == h)).unwrap_or(0);
    let theme = dialoguer::theme::SimpleTheme;
    let Some(i) = dialoguer::Select::with_theme(&theme)
        .with_prompt("Launch which profile? (Esc to quit)")
        .items(&labels)
        .default(default)
        .interact_opt()?
    else {
        return Ok(0);
    };
    let p = &profiles[i];
    let tool = if p.file.tools.len() > 1 {
        let names: Vec<&str> = p.file.tools.iter().map(|t| t.as_str()).collect();
        match dialoguer::Select::with_theme(&theme).with_prompt("Which CLI?").items(&names).default(0).interact_opt()? {
            Some(j) => p.file.tools[j],
            None => return Ok(0),
        }
    } else {
        p.file.tools[0]
    };
    launch::run(p, tool, &[])
}
