//! keepapart: one machine, many Claude & Codex accounts, kept apart.
//!
//! Every profile is a pair of folders handed to the official CLIs through
//! their documented variables (CLAUDE_CONFIG_DIR, CODEX_HOME). The official
//! CLIs log in and store their own credentials there. keepapart never reads,
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
