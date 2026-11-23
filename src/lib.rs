//! keepapart: one machine, many AI coding accounts (Claude Code, Codex, Gemini CLI, Grok, Kimi, ...), kept apart.
//!
//! Every profile gives each AI CLI its own folder, handed over through the
//! tool's documented variable (CLAUDE_CONFIG_DIR, CODEX_HOME, GEMINI_CLI_HOME,
//! ...; see [`tools`]). The official CLIs log in and store their own
//! credentials there. keepapart never reads,
//! copies or moves a credential, never touches a keychain, and makes no
//! network calls.

pub mod doctor;
pub mod export;
pub mod launch;
pub mod mcp;
pub mod paths;
pub mod profile;
pub mod resolve;
pub mod share;
pub mod shell;
pub mod tools;
