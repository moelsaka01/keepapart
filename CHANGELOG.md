# Changelog

## 0.1.1 (2026-11-15)

- `keepapart list` on macOS shows `claude:?` instead of `claude:keychain`: Claude Code keeps macOS logins in the Keychain, which keepapart never reads, so it can't know. It now says how to check.
- Shared items that are copies (Windows without symlink rights) keep their modified time when copied, so newest-wins sync can't let an older edit win on Linux and macOS.
- This repo is now its own Scoop bucket (`bucket/`) and Homebrew tap (`Formula/`). Each release updates both, then CI installs through them on fresh Windows, macOS and Linux machines.
- First release on crates.io (`cargo install keepapart`) and winget.

## 0.1.0 (2026-11-12)

First release: isolated Claude Code and Codex profiles (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`), per-folder `.keepapart` files, shared or per-profile skills, agents, commands and settings, MCP sharing through `claude mcp add-json`, shell hooks for bash, zsh, fish and PowerShell, `doctor`, `export-config`, and the picker you get with no arguments. Tested live on Windows 11 with Claude Code and Codex; CI on Windows, macOS and Linux.
