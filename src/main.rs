use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use keepapart::paths::{ago, now_secs};
use keepapart::profile::{Profile, Store, Tool, ToolChoice};
use keepapart::resolve::{self, Resolved, Source};
use keepapart::share::{self, ItemState, ShareOutcome, CATALOG};
use keepapart::shell::{self, Desired, Shell};
use keepapart::{doctor, export, launch, mcp};
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
        /// Extra arguments for the login command, after -- (e.g. -- --sso)
        #[arg(last = true)]
        args: Vec<OsString>,
    },
    /// Launch claude or codex with a profile. Several can run at once.
    #[command(alias = "r")]
    Run {
        /// Profile (default: the one for this folder, see `keepapart status`)
        name: Option<String>,
        /// Which CLI, for profiles that have both (default: claude)
        #[arg(long, short, value_enum)]
        tool: Option<Tool>,
        /// Hide ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN / CLAUDE_CODE_OAUTH_TOKEN from this launch,
        /// so the profile's own login is used
        #[arg(long)]
        clean_env: bool,
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
        /// Don't sign out through the official CLI before deleting
        #[arg(long)]
        keep_login: bool,
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
    /// Refresh shared items that are copies (Windows without symlink rights): newest wins
    Sync,
    /// Share Claude Code MCP servers across profiles (installed with `claude mcp add-json`)
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
    /// Check installs, PATH, isolation and link permissions
    Doctor,
    /// Print a shell hook that applies the folder's profile to plain claude/codex
    Hook {
        #[arg(value_enum)]
        shell: Shell,
        /// Save the hook to a file in keepapart's folder and print the line to add to your shell profile
        #[arg(long)]
        write: bool,
    },
    /// Print env commands for the current folder's profile (used by the hook)
    Env {
        #[arg(long, value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// Share a server: keepapart mcp add github '{"command":"npx","args":["-y","@modelcontextprotocol/server-github"]}'
    Add { name: String, json: String },
    /// Stop sharing a server (removed from every profile keepapart added it to)
    Remove { name: String },
    /// List shared servers
    List,
    /// Install the shared servers into every Claude profile (or one)
    Sync { profile: Option<String> },
}

fn main() {
    // `keepapart list | head` closes the pipe early; exit quietly instead of panicking.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("failed printing to std") {
            std::process::exit(0);
        }
        default_hook(info);
    }));
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
        Some(Cmd::Login { name, tool, args }) => {
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
                login(&p, t, &args)?;
            }
            Ok(0)
        }
        Some(Cmd::Run { name, tool, clean_env, args }) => {
            let r = need_profile(&store, name.as_deref(), &cwd)?;
            let p = store.find(&r.name).with_context(|| format!("profile {}", r.source.describe()))?;
            let t = pick_tool(&p, tool)?;
            if r.source != Source::Explicit {
                eprintln!("keepapart: {} as {:?} ({})", t.as_str(), p.name(), r.source.describe());
            }
            launch_profile(&store, &p, t, &args, clean_env)
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
        Some(Cmd::Remove { name, yes, keep_login }) => remove(&store, &name, yes, keep_login),
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
        Some(Cmd::Sync) => {
            let report = share::sync_all(&store)?;
            if report.is_empty() {
                println!("All shared copies are up to date.");
            }
            for (p, items) in report {
                println!("{p}: refreshed {}", items.join(", "));
            }
            Ok(0)
        }
        Some(Cmd::Mcp { cmd }) => mcp_cmd(&store, cmd),
        Some(Cmd::Doctor) => {
            let d = doctor::run(&store)?;
            Ok(if d.errors > 0 { 1 } else { 0 })
        }
        Some(Cmd::Hook { shell, write: true }) => {
            let path = store.home.join(shell::hook_file_name(shell));
            keepapart::paths::write_atomic(&path, shell::hook(shell).as_bytes())?;
            println!("Wrote {}", path.display());
            let line = match shell {
                Shell::Powershell | Shell::PsData => format!(". '{}'", path.display().to_string().replace('\'', "''")),
                Shell::Fish => format!("source {}", shell::quote(Shell::Fish, &path.display().to_string())),
                _ => format!(". {}", shell::quote(Shell::Bash, &path.display().to_string())),
            };
            let profile = match shell {
                Shell::Bash => "~/.bashrc",
                Shell::Zsh => "~/.zshrc",
                Shell::Fish => "~/.config/fish/config.fish",
                Shell::Powershell | Shell::PsData => "your PowerShell profile (notepad $PROFILE)",
            };
            println!("Add this line to {profile}:\n\n    {line}\n");
            Ok(0)
        }
        Some(Cmd::Hook { shell, write: false }) => {
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

fn login(p: &Profile, t: Tool, extra: &[OsString]) -> Result<()> {
    let mut args = launch::login_args(t);
    args.extend(extra.iter().cloned());
    println!();
    println!(
        "Running `{} {}` as profile {:?}. Sign in with the account for this profile.",
        t.program(),
        args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "),
        p.name()
    );
    println!("Login is handled entirely by the official CLI. keepapart never sees it.\n");
    let code = launch::run_wait(p, t, &args)?;
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
    if !isolated && p.has(Tool::Claude) && !mcp::load(store)?.is_empty() {
        if launch::program_path(Tool::Claude).is_ok() {
            match mcp::sync_profile(store, &mut p) {
                Ok(log) => println!("Shared MCP servers: {}", log.join(", ")),
                Err(e) => eprintln!("keepapart: couldn't install shared MCP servers: {e:#}"),
            }
        } else {
            println!("Shared MCP servers will be installed by `keepapart mcp sync` once claude is installed.");
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
        login(&p, t, &[])?;
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

fn remove(store: &Store, name: &str, yes: bool, keep_login: bool) -> Result<i32> {
    let p = store.find(name)?;
    println!("This deletes {}", p.dir.display());
    println!("including profile {name:?}'s login files and session history. Shared items are untouched.");
    if keep_login {
        println!("--keep-login: the official CLIs won't be asked to sign out first.");
    } else {
        println!("First, the official CLIs sign this profile out (`claude auth logout`, `codex logout`),");
        println!(
            "which also clears any OS keychain entry they made for it. keepapart never touches the keychain itself."
        );
    }
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
    if !keep_login {
        for &t in &p.file.tools {
            let signed_in = p.login_file_present(t) || (t == Tool::Claude && cfg!(target_os = "macos"));
            if !signed_in {
                continue;
            }
            match launch::capture(&p, t, &launch::logout_args(t)) {
                Ok(o) if o.status.success() => println!("Signed out of {} for {name:?}.", t.as_str()),
                Ok(o) => {
                    eprintln!("keepapart: {} logout said: {}", t.as_str(), String::from_utf8_lossy(&o.stderr).trim())
                }
                Err(e) => eprintln!("keepapart: couldn't run {} to sign out ({e:#}); deleting anyway", t.as_str()),
            }
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
    launch_profile(store, p, tool, &[], false)
}

/// Launch a profile: refresh its copied shared items first (a no-op where
/// shared items are links), then hand over to the official CLI.
fn launch_profile(store: &Store, p: &Profile, t: Tool, args: &[OsString], clean_env: bool) -> Result<i32> {
    match share::sync_copies(store, p) {
        Ok(ch) if !ch.is_empty() => eprintln!("keepapart: refreshed shared copies: {}", ch.join(", ")),
        Ok(_) => {}
        Err(e) => eprintln!("keepapart: couldn't refresh shared copies: {e:#}"),
    }
    if let Ok((path, launch::Found::InstallerDir)) = launch::locate(t) {
        eprintln!("keepapart: note: {}", launch::path_advice(t, &path));
    }
    launch::run(p, t, args, clean_env)
}

fn mcp_sync(store: &Store, only: Option<&str>) -> Result<i32> {
    let mut profiles = match only {
        Some(n) => vec![store.find(n)?],
        None => store.list()?,
    };
    profiles.retain(|p| p.has(Tool::Claude));
    if profiles.is_empty() {
        println!("No Claude profiles to update.");
        return Ok(0);
    }
    if launch::program_path(Tool::Claude).is_err() {
        bail!(
            "`claude` isn't installed, so profiles can't be updated yet. Run `keepapart mcp sync` after installing it"
        );
    }
    for mut p in profiles {
        let log = mcp::sync_profile(store, &mut p)?;
        println!("  {}: {}", p.name(), if log.is_empty() { "nothing to do".into() } else { log.join(", ") });
    }
    Ok(0)
}

fn mcp_cmd(store: &Store, cmd: McpCmd) -> Result<i32> {
    match cmd {
        McpCmd::Add { name, json } => {
            mcp::add(store, &name, &json)?;
            println!(
                "Sharing MCP server {name:?}. Installing it in each Claude profile (claude mcp add-json --scope user):"
            );
            mcp_sync(store, None)
        }
        McpCmd::Remove { name } => {
            if !mcp::remove(store, &name)? {
                bail!("no shared MCP server named {name:?}");
            }
            println!("Stopped sharing {name:?}. Removing it from the profiles keepapart added it to:");
            mcp_sync(store, None)
        }
        McpCmd::List => {
            let m = mcp::load(store)?;
            if m.is_empty() {
                println!("No shared MCP servers. Add one: keepapart mcp add <name> '<json>'");
            }
            for (name, cfg) in m {
                let what = cfg.get("command").or_else(|| cfg.get("url")).and_then(|v| v.as_str()).unwrap_or("?");
                println!("  {name:20} {what}");
            }
            println!("(stored in {})", mcp::path(store).display());
            Ok(0)
        }
        McpCmd::Sync { profile } => mcp_sync(store, profile.as_deref()),
    }
}
