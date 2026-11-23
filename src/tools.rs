//! The AI CLIs keepapart knows how to keep apart.
//!
//! A tool qualifies only if it has an official, documented way to move its
//! whole state, login included, into another folder. keepapart points that
//! setting at a per-profile folder and lets the tool's own login command write
//! its own credentials there. Tools whose login lives in an OS keychain entry
//! with a fixed name are only included when they also offer an official switch
//! to keep it in that folder instead (Gemini CLI, Goose).
//!
//! Built-ins are below. Anyone can add more in `~/.keepapart/tools.toml`
//! (see [`load_custom`]); the same rules are checked there.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// The tool's folder in the profile, or a sub-folder of it ("" = the folder itself).
    Dir(&'static str),
    /// A fixed value, e.g. "true" for a switch.
    Lit(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct EnvVar {
    pub var: &'static str,
    pub value: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

/// Something a profile can share with other profiles (skills, instructions...).
#[derive(Clone, Copy, Debug)]
pub struct ShareDef {
    /// Short name used in `tool:name` ids.
    pub name: &'static str,
    /// Path inside the tool's folder.
    pub path: &'static str,
    pub kind: Kind,
    pub default: bool,
    pub about: &'static str,
}

#[derive(Debug)]
pub struct ToolSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub program: &'static str,
    /// "subscription", "open source", ... (shown in `keepapart tools`).
    pub kind: &'static str,
    pub env: &'static [EnvVar],
    /// May the shell hook export these variables into your shell? False when a
    /// variable would also affect unrelated programs (XDG_* for OpenCode).
    pub hook: bool,
    /// Arguments for the official login command. `None`: the tool has no login
    /// command (API keys in a file the tool reads); `login_hint` says what to do.
    pub login: Option<&'static [&'static str]>,
    pub login_hint: &'static str,
    pub logout: Option<&'static [&'static str]>,
    /// Files the tool writes its login to, relative to its folder. keepapart only
    /// checks whether one exists. Empty: unknown.
    pub login_files: &'static [&'static str],
    /// On macOS the login lives in the Keychain (keyed to the folder), so "no file" proves nothing.
    pub macos_keychain: bool,
    /// Variables that make the tool ignore the profile's login.
    pub overriding: &'static [&'static str],
    /// The tool's normal folders, relative to the home folder (doctor checks a profile never uses them).
    pub default_dirs: &'static [&'static str],
    /// Paths inside the folder that hold logins or history and must never be shared.
    pub never_shared: &'static [&'static str],
    pub share: &'static [ShareDef],
    pub install: &'static str,
    /// Where the isolation variable is documented.
    pub source: &'static str,
    pub custom: bool,
}

/// A tool keepapart can manage. Cheap to copy; compares by id.
#[derive(Clone, Copy)]
pub struct Tool(pub &'static ToolSpec);

impl Tool {
    pub fn spec(self) -> &'static ToolSpec {
        self.0
    }
    pub fn as_str(self) -> &'static str {
        self.0.id
    }
    pub fn program(self) -> &'static str {
        self.0.program
    }
    pub fn parse(s: &str) -> Option<Tool> {
        all().iter().copied().find(|t| t.0.id == s)
    }
    /// The first variable that points at the tool's folder (used in messages).
    pub fn main_var(self) -> &'static str {
        self.0.env.iter().find(|e| matches!(e.value, Value::Dir(_))).map(|e| e.var).unwrap_or("")
    }
    pub const CLAUDE: Tool = Tool(&CLAUDE);
    pub const CODEX: Tool = Tool(&CODEX);
}

impl PartialEq for Tool {
    fn eq(&self, o: &Self) -> bool {
        self.0.id == o.0.id
    }
}
impl Eq for Tool {}
impl std::hash::Hash for Tool {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.0.id.hash(h)
    }
}
impl PartialOrd for Tool {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Tool {
    /// Built-in order first (Claude, Codex, ...), then custom tools by id.
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        let pos = |t: &Tool| all().iter().position(|x| x == t).unwrap_or(usize::MAX);
        pos(self).cmp(&pos(o)).then(self.0.id.cmp(o.0.id))
    }
}
impl fmt::Debug for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tool({})", self.0.id)
    }
}
impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.id)
    }
}
impl serde::Serialize for Tool {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.0.id)
    }
}
impl<'de> Deserialize<'de> for Tool {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Tool::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown tool {s:?}; add it to tools.toml or update keepapart"))
        })
    }
}

/// clap value parser for `--tool`.
pub fn parse_tool(s: &str) -> std::result::Result<Tool, String> {
    Tool::parse(s).ok_or_else(|| {
        let ids: Vec<&str> = all().iter().map(|t| t.as_str()).collect();
        format!("unknown tool {s:?}. Known: {}", ids.join(", "))
    })
}

/// `--tool claude,gemini` / `both` (claude+codex).
pub fn parse_tool_list(s: &str) -> Result<Vec<Tool>> {
    let mut out = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if part == "both" {
            out.extend([Tool::CLAUDE, Tool::CODEX]);
        } else {
            out.push(parse_tool(part).map_err(anyhow::Error::msg)?);
        }
    }
    if out.is_empty() {
        bail!("no tool given");
    }
    out.sort();
    out.dedup();
    Ok(out)
}

const fn dir(var: &'static str) -> EnvVar {
    EnvVar { var, value: Value::Dir("") }
}
const fn sub(var: &'static str, path: &'static str) -> EnvVar {
    EnvVar { var, value: Value::Dir(path) }
}
const fn lit(var: &'static str, v: &'static str) -> EnvVar {
    EnvVar { var, value: Value::Lit(v) }
}
const fn share(name: &'static str, path: &'static str, kind: Kind, default: bool, about: &'static str) -> ShareDef {
    ShareDef { name, path, kind, default, about }
}

const NONE: &[&str] = &[];

pub const CLAUDE: ToolSpec = ToolSpec {
    id: "claude",
    name: "Claude Code",
    program: "claude",
    kind: "subscription (Claude Pro/Max/Team) or API key",
    env: &[dir("CLAUDE_CONFIG_DIR")],
    hook: true,
    login: Some(&["auth", "login"]),
    login_hint: "",
    logout: Some(&["auth", "logout"]),
    login_files: &[".credentials.json"],
    macos_keychain: true,
    overriding: &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"],
    default_dirs: &[".claude"],
    never_shared: &[
        ".credentials.json",
        ".claude.json",
        "projects",
        "history.jsonl",
        "sessions",
        "todos",
        "file-history",
        "shell-snapshots",
        "statsig",
    ],
    share: &[
        share("skills", "skills", Kind::Dir, true, "Agent skills"),
        share("agents", "agents", Kind::Dir, true, "Subagent definitions"),
        share("commands", "commands", Kind::Dir, true, "Custom slash commands"),
        share("output-styles", "output-styles", Kind::Dir, true, "Output styles"),
        share("CLAUDE.md", "CLAUDE.md", Kind::File, true, "Your global instructions"),
        share(
            "settings.json",
            "settings.json",
            Kind::File,
            false,
            "Settings: model, permissions, hooks, env, statusline",
        ),
        share("plugins", "plugins", Kind::Dir, false, "Plugins and marketplaces (can bundle MCP servers)"),
    ],
    install: "https://code.claude.com/docs/en/setup",
    source: "https://code.claude.com/docs/en/authentication",
    custom: false,
};

pub const CODEX: ToolSpec = ToolSpec {
    id: "codex",
    name: "Codex CLI",
    program: "codex",
    kind: "subscription (ChatGPT) or API key",
    env: &[dir("CODEX_HOME")],
    hook: true,
    login: Some(&["login"]),
    login_hint: "",
    logout: Some(&["logout"]),
    login_files: &["auth.json"],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[".codex"],
    never_shared: &["auth.json", "sessions", "archived_sessions", "history.jsonl", "log"],
    share: &[
        share("skills", "skills", Kind::Dir, true, "Agent skills"),
        share("prompts", "prompts", Kind::Dir, true, "Custom prompts"),
        share("AGENTS.md", "AGENTS.md", Kind::File, true, "Your global instructions"),
        share("config.toml", "config.toml", Kind::File, false, "Settings and MCP servers"),
        share("rules", "rules", Kind::Dir, false, "Command approval rules"),
    ],
    install: "https://developers.openai.com/codex/cli",
    source: "https://developers.openai.com/codex/auth",
    custom: false,
};

pub const GEMINI: ToolSpec = ToolSpec {
    id: "gemini",
    name: "Gemini CLI",
    program: "gemini",
    kind: "subscription (Google account) or API key; open source",
    // GEMINI_CLI_HOME replaces the home folder; Gemini uses <it>/.gemini. Its
    // optional encrypted storage would put the login in a keychain entry with a
    // fixed name, so GEMINI_FORCE_FILE_STORAGE keeps it in that folder instead.
    env: &[dir("GEMINI_CLI_HOME"), lit("GEMINI_FORCE_FILE_STORAGE", "true")],
    hook: true,
    login: Some(NONE),
    login_hint: "Gemini CLI asks you to sign in on first start (or type /auth).",
    logout: None,
    login_files: &[".gemini/oauth_creds.json", ".gemini/gemini-credentials.json"],
    macos_keychain: false,
    overriding: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
    default_dirs: &[".gemini"],
    never_shared: &[
        ".gemini/oauth_creds.json",
        ".gemini/gemini-credentials.json",
        ".gemini/google_accounts.json",
        ".gemini/tmp",
        ".gemini/history",
    ],
    share: &[
        share("GEMINI.md", ".gemini/GEMINI.md", Kind::File, true, "Your global instructions"),
        share("commands", ".gemini/commands", Kind::Dir, true, "Custom commands"),
        share("settings.json", ".gemini/settings.json", Kind::File, false, "Settings and MCP servers"),
    ],
    install: "https://github.com/google-gemini/gemini-cli",
    source: "https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/enterprise.md",
    custom: false,
};

pub const QWEN: ToolSpec = ToolSpec {
    id: "qwen",
    name: "Qwen Code",
    program: "qwen",
    kind: "Qwen account or API key; open source",
    env: &[dir("QWEN_HOME")],
    hook: true,
    login: Some(NONE),
    login_hint: "Qwen Code asks you to sign in on first start (or type /auth).",
    logout: None,
    login_files: &["oauth_creds.json"],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[".qwen"],
    never_shared: &["oauth_creds.json", "tmp", "history"],
    share: &[
        share("QWEN.md", "QWEN.md", Kind::File, true, "Your global instructions"),
        share("settings.json", "settings.json", Kind::File, false, "Settings and MCP servers"),
    ],
    install: "https://github.com/QwenLM/qwen-code",
    source: "https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/config/storage.ts",
    custom: false,
};

pub const GROK: ToolSpec = ToolSpec {
    id: "grok",
    name: "Grok Build (xAI)",
    program: "grok",
    kind: "subscription (SuperGrok) or API key; open source",
    env: &[dir("GROK_HOME")],
    hook: true,
    login: Some(&["login"]),
    login_hint: "",
    logout: Some(&["logout"]),
    login_files: &["auth.json"],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[".grok"],
    never_shared: &["auth.json", "mcp_credentials.json", "sessions"],
    share: &[share("config.toml", "config.toml", Kind::File, false, "Settings")],
    install: "https://docs.x.ai/build/overview",
    source: "https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/26-config-reference.md",
    custom: false,
};

pub const KIMI: ToolSpec = ToolSpec {
    id: "kimi",
    name: "Kimi Code CLI",
    program: "kimi",
    kind: "subscription (Kimi Code) or API key",
    env: &[dir("KIMI_CODE_HOME")],
    hook: true,
    login: Some(&["login"]),
    login_hint: "",
    logout: None,
    login_files: &[],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[".kimi-code"],
    never_shared: &["sessions", "logs"],
    share: &[share("config.toml", "config.toml", Kind::File, false, "Settings")],
    install: "https://moonshotai.github.io/kimi-code/en/guides/getting-started",
    source: "https://moonshotai.github.io/kimi-code/en/configuration/env-vars.html",
    custom: false,
};

pub const GOOSE: ToolSpec = ToolSpec {
    id: "goose",
    name: "Goose",
    program: "goose",
    kind: "open source, any model",
    // GOOSE_PATH_ROOT moves config/data/state. Goose's keyring entry has a
    // fixed name, so GOOSE_DISABLE_KEYRING keeps secrets in <root>/config/secrets.yaml.
    env: &[dir("GOOSE_PATH_ROOT"), lit("GOOSE_DISABLE_KEYRING", "1")],
    hook: true,
    login: Some(&["configure"]),
    login_hint: "",
    logout: None,
    login_files: &["config/secrets.yaml"],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[],
    never_shared: &["config/secrets.yaml", "data", "state"],
    share: &[share("config.yaml", "config/config.yaml", Kind::File, false, "Settings and extensions")],
    install: "https://github.com/block/goose",
    source: "https://github.com/block/goose/blob/main/crates/goose/src/config/paths.rs",
    custom: false,
};

pub const CRUSH: ToolSpec = ToolSpec {
    id: "crush",
    name: "Crush",
    program: "crush",
    kind: "open source, any model",
    env: &[sub("CRUSH_GLOBAL_CONFIG", "config"), sub("CRUSH_GLOBAL_DATA", "data")],
    hook: true,
    login: Some(NONE),
    login_hint: "Crush asks for a provider and key on first start.",
    logout: None,
    login_files: &[],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[],
    never_shared: &["data"],
    share: &[share("crush.json", "config/crush.json", Kind::File, false, "Settings, providers and MCP servers")],
    install: "https://github.com/charmbracelet/crush",
    source: "https://github.com/charmbracelet/crush#readme",
    custom: false,
};

pub const CLINE: ToolSpec = ToolSpec {
    id: "cline",
    name: "Cline CLI",
    program: "cline",
    kind: "Cline account or API key; open source",
    env: &[dir("CLINE_DIR"), sub("CLINE_DATA_DIR", "data")],
    hook: true,
    login: Some(&["auth"]),
    login_hint: "",
    logout: None,
    login_files: &["data/settings/providers.json"],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[".cline"],
    never_shared: &["data"],
    share: &[],
    install: "https://docs.cline.bot/",
    source: "https://docs.cline.bot/cli/cli-reference",
    custom: false,
};

pub const CONTINUE: ToolSpec = ToolSpec {
    id: "continue",
    name: "Continue CLI",
    program: "cn",
    kind: "Continue account or API key; open source",
    env: &[dir("CONTINUE_GLOBAL_DIR")],
    hook: true,
    login: Some(NONE),
    login_hint: "Continue CLI asks you to sign in on first start (or type /login).",
    logout: None,
    login_files: &[],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[".continue"],
    never_shared: &["sessions", "index"],
    share: &[share("config.yaml", "config.yaml", Kind::File, false, "Settings, models and MCP servers")],
    install: "https://docs.continue.dev/",
    source: "https://github.com/continuedev/continue/blob/main/core/util/paths.ts",
    custom: false,
};

pub const AIDER: ToolSpec = ToolSpec {
    id: "aider",
    name: "Aider",
    program: "aider",
    kind: "open source, API keys",
    // Aider loads ~/.env first, then this file (later files win).
    env: &[sub("AIDER_ENV_FILE", ".env")],
    hook: true,
    login: None,
    login_hint: "Aider has no login: put this profile's API keys in {dir}/.env (Aider reads it; keepapart never does).",
    logout: None,
    login_files: &[".env"],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[],
    never_shared: &[".env"],
    share: &[],
    install: "https://aider.chat/docs/install.html",
    source: "https://aider.chat/docs/config/dotenv.html",
    custom: false,
};

pub const OPENCODE: ToolSpec = ToolSpec {
    id: "opencode",
    name: "OpenCode",
    program: "opencode",
    kind: "open source, any model (incl. ChatGPT / Copilot logins)",
    // OpenCode has no variable of its own; it follows the XDG base folders on
    // every OS. Data (logins, sessions) and state go per profile; config stays
    // shared. Those variables would also affect other programs, so the shell
    // hook doesn't export them: use `keepapart run`.
    env: &[sub("XDG_DATA_HOME", "data"), sub("XDG_STATE_HOME", "state")],
    hook: false,
    login: Some(&["auth", "login"]),
    login_hint: "",
    logout: Some(&["auth", "logout"]),
    login_files: &["data/opencode/auth.json"],
    macos_keychain: false,
    overriding: &[],
    default_dirs: &[],
    never_shared: &["data", "state"],
    share: &[],
    install: "https://opencode.ai/docs/",
    source: "https://github.com/anomalyco/opencode/blob/dev/packages/util/src/global-roots.ts",
    custom: false,
};

const BUILTIN: &[&ToolSpec] =
    &[&CLAUDE, &CODEX, &GEMINI, &QWEN, &GROK, &KIMI, &GOOSE, &CRUSH, &CLINE, &CONTINUE, &AIDER, &OPENCODE];

/// Tools that were checked and can't be kept apart safely (shown by `keepapart tools`).
pub const UNSUPPORTED: &[(&str, &str)] = &[
    ("GitHub Copilot CLI", "keeps its login in the OS keychain under a fixed name; COPILOT_HOME doesn't move it. It has its own account switcher: /user switch"),
    ("Cursor CLI", "no documented way to move where it stores its login"),
    ("Amp", "no documented way to move where it stores its login"),
    ("Kiro CLI", "no documented way to move where it stores its login"),
    ("Codewhale (ex DeepSeek-TUI)", "keeps keys in the OS keyring under a fixed name (\"deepseek\")"),
];

static REGISTRY: OnceLock<Vec<Tool>> = OnceLock::new();

/// Built-ins plus the custom tools in `<keepapart home>/tools.toml`.
/// The first call decides; later calls return the same list.
pub fn init(home: Option<&Path>) -> Result<&'static [Tool]> {
    if let Some(r) = REGISTRY.get() {
        return Ok(r);
    }
    let mut tools: Vec<Tool> = BUILTIN.iter().map(|s| Tool(s)).collect();
    if let Some(home) = home {
        let path = home.join("tools.toml");
        if path.is_file() {
            let text = std::fs::read_to_string(&path)?;
            tools.extend(load_custom(&text).with_context(|| format!("in {}", path.display()))?);
        }
    }
    Ok(REGISTRY.get_or_init(|| tools))
}

pub fn all() -> &'static [Tool] {
    REGISTRY.get_or_init(|| BUILTIN.iter().map(|s| Tool(s)).collect())
}

/// Every variable any known tool sets (the shell hook manages these).
pub fn hook_vars() -> Vec<&'static str> {
    let mut v: Vec<&str> = all().iter().filter(|t| t.0.hook).flat_map(|t| t.0.env.iter().map(|e| e.var)).collect();
    v.sort();
    v.dedup();
    v
}

/// Variables keepapart will never set, even for a custom tool.
const FORBIDDEN_VARS: &[&str] = &[
    "PATH",
    "PATHEXT",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
    "TMPDIR",
    "SHELL",
    "COMSPEC",
    "SYSTEMROOT",
    "WINDIR",
    "PSMODULEPATH",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "NODE_OPTIONS",
    "PYTHONPATH",
    "KEEPAPART_ACTIVE",
    "KEEPAPART_PROFILE",
    "KEEPAPART_HOME",
];

pub fn valid_var_name(v: &str) -> bool {
    !v.is_empty()
        && v.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && v.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !FORBIDDEN_VARS.contains(&v)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomFile {
    #[serde(default)]
    tool: Vec<CustomTool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomTool {
    id: String,
    name: Option<String>,
    program: Option<String>,
    /// VAR = "{dir}", "{dir}/sub" or a fixed value.
    env: HashMap<String, String>,
    #[serde(default = "yes")]
    hook: bool,
    login: Option<Vec<String>>,
    #[serde(default)]
    login_hint: String,
    logout: Option<Vec<String>>,
    #[serde(default)]
    login_files: Vec<String>,
    #[serde(default)]
    override_vars: Vec<String>,
    #[serde(default)]
    share: Vec<CustomShare>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomShare {
    name: String,
    path: Option<String>,
    kind: String,
    #[serde(default)]
    default: bool,
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_list(v: Vec<String>) -> &'static [&'static str] {
    Box::leak(v.into_iter().map(leak).collect::<Vec<_>>().into_boxed_slice())
}

fn safe_rel_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.starts_with('\\')
        && !p.contains(':')
        && p.split(['/', '\\']).all(|c| !c.is_empty() && c != "." && c != "..")
}

/// Parse `tools.toml`. Each custom tool must point at least one variable at its
/// profile folder (`{dir}`), or it wouldn't be kept apart at all.
pub fn load_custom(text: &str) -> Result<Vec<Tool>> {
    let file: CustomFile = toml::from_str(text)?;
    let mut out = Vec::new();
    for t in file.tool {
        crate::paths::validate_name(&t.id).with_context(|| format!("tool id {:?}", t.id))?;
        if BUILTIN.iter().any(|b| b.id == t.id) || out.iter().any(|o: &Tool| o.0.id == t.id) {
            bail!("tool {:?} is defined twice (or is built in)", t.id);
        }
        let mut env = Vec::new();
        let mut has_dir = false;
        let mut vars: Vec<_> = t.env.into_iter().collect();
        vars.sort();
        for (var, value) in vars {
            if !valid_var_name(&var) {
                bail!("tool {:?}: {var:?} can't be set (use an UPPER_CASE variable the tool documents)", t.id);
            }
            let value = if value == "{dir}" {
                has_dir = true;
                Value::Dir("")
            } else if let Some(rest) = value.strip_prefix("{dir}/") {
                if !safe_rel_path(rest) {
                    bail!("tool {:?}: {var} = {value:?} must stay inside the profile folder", t.id);
                }
                has_dir = true;
                Value::Dir(leak(rest.to_string()))
            } else if value.contains('{') {
                bail!("tool {:?}: {var} = {value:?}: only {{dir}} and {{dir}}/sub are allowed", t.id);
            } else {
                Value::Lit(leak(value))
            };
            env.push(EnvVar { var: leak(var), value });
        }
        if !has_dir {
            bail!(
                "tool {:?}: no variable points at the profile folder ({{dir}}), so its login wouldn't be kept apart",
                t.id
            );
        }
        let mut share = Vec::new();
        for s in t.share {
            let path = s.path.unwrap_or_else(|| s.name.clone());
            if !safe_rel_path(&path) || s.name.contains(':') {
                bail!("tool {:?}: share item {:?} must be a path inside the profile folder", t.id, s.name);
            }
            let kind = match s.kind.as_str() {
                "file" => Kind::File,
                "dir" => Kind::Dir,
                k => bail!("tool {:?}: share kind {k:?} must be \"file\" or \"dir\"", t.id),
            };
            share.push(ShareDef { name: leak(s.name), path: leak(path), kind, default: s.default, about: "custom" });
        }
        for f in &t.login_files {
            if !safe_rel_path(f) {
                bail!("tool {:?}: login file {f:?} must be a path inside the profile folder", t.id);
            }
        }
        for v in &t.override_vars {
            if !valid_var_name(v) {
                bail!("tool {:?}: override_vars entry {v:?} isn't a variable name", t.id);
            }
        }
        let login_files = leak_list(t.login_files);
        let spec = ToolSpec {
            id: leak(t.id.clone()),
            name: leak(t.name.unwrap_or_else(|| t.id.clone())),
            program: leak(t.program.unwrap_or_else(|| t.id.clone())),
            kind: "custom",
            env: Box::leak(env.into_boxed_slice()),
            hook: t.hook,
            login: t.login.map(leak_list),
            login_hint: leak(t.login_hint),
            logout: t.logout.map(leak_list),
            login_files,
            macos_keychain: false,
            overriding: leak_list(t.override_vars),
            default_dirs: &[],
            never_shared: login_files,
            share: Box::leak(share.into_boxed_slice()),
            install: "",
            source: "tools.toml",
            custom: true,
        };
        out.push(Tool(Box::leak(Box::new(spec))));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_points_a_variable_at_its_folder() {
        for s in BUILTIN {
            assert!(s.env.iter().any(|e| matches!(e.value, Value::Dir(_))), "{}", s.id);
            for e in s.env {
                assert!(valid_var_name(e.var), "{}: {}", s.id, e.var);
            }
            for sh in s.share {
                assert!(safe_rel_path(sh.path), "{}: {}", s.id, sh.path);
                assert!(!s.never_shared.iter().any(|n| sh.path == *n || sh.path.starts_with(&format!("{n}/"))));
            }
            crate::paths::validate_name(s.id).unwrap();
        }
        let ids: std::collections::HashSet<_> = BUILTIN.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), BUILTIN.len());
    }

    #[test]
    fn custom_tools_are_checked() {
        let ok = load_custom(
            r#"
[[tool]]
id = "mytool"
env = { MYTOOL_HOME = "{dir}", MYTOOL_NO_KEYRING = "1", MYTOOL_DATA = "{dir}/data" }
login = ["login"]
login_files = ["auth.json"]
share = [{ name = "AGENTS.md", kind = "file", default = true }]
"#,
        )
        .unwrap();
        let t = ok[0];
        assert_eq!(t.program(), "mytool");
        assert_eq!(t.spec().env.len(), 3);
        assert_eq!(t.spec().never_shared, &["auth.json"]);

        for bad in [
            r#"[[tool]]
id = "x"
env = { X_MODE = "1" }"#, // nothing points at the folder
            r#"[[tool]]
id = "x"
env = { PATH = "{dir}" }"#,
            r#"[[tool]]
id = "x"
env = { X_HOME = "{dir}/../escape" }"#,
            r#"[[tool]]
id = "claude"
env = { X_HOME = "{dir}" }"#,
            r#"[[tool]]
id = "Bad Name"
env = { X_HOME = "{dir}" }"#,
            r#"[[tool]]
id = "x"
env = { X_HOME = "{home}" }"#,
            r#"[[tool]]
id = "x"
env = { X_HOME = "{dir}" }
share = [{ name = "a", path = "../../etc", kind = "file" }]"#,
            r#"[[tool]]
id = "x"
env = { X_HOME = "{dir}" }
typo = 1"#,
        ] {
            assert!(load_custom(bad).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn tool_lists_parse() {
        assert_eq!(parse_tool_list("both").unwrap(), vec![Tool::CLAUDE, Tool::CODEX]);
        let l = parse_tool_list("gemini, claude,gemini").unwrap();
        assert_eq!(l.iter().map(|t| t.as_str()).collect::<Vec<_>>(), vec!["claude", "gemini"]);
        assert!(parse_tool_list("nope").is_err());
    }
}
