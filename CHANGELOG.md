# Changelog

## 0.2.0 (unreleased)

- **Not just Claude and Codex anymore.** Built-in support for Gemini CLI, Grok Build (xAI), Kimi Code CLI, Qwen Code, Goose, Crush, Cline CLI, Continue CLI, Aider and OpenCode, each through the setting its makers document for moving its folder (`GEMINI_CLI_HOME`, `GROK_HOME`, `KIMI_CODE_HOME`, ...). Where a tool could fall back to a keychain entry with a fixed name (Gemini CLI, Goose), keepapart also sets the tool's own switch that keeps the login in the profile's folder.
- **Your own tools**: define any other CLI in `~/.keepapart/tools.toml`. Definitions that wouldn't keep logins apart, set system variables or leave the profile folder are refused.
- `keepapart tools` lists every supported CLI, what keepapart sets for it, and which are installed, plus the ones checked and not supported (GitHub Copilot CLI, Cursor, Amp, Kiro, Codewhale) and why.
- `--tool` takes a list: `keepapart add work --tool claude,gemini,grok`.
- The shell hooks handle any number of tools. OpenCode's XDG variables are never exported to your shell.
- `export-config` skips YAML settings files, which it can't check for secrets.

## 0.1.1 (2026-11-15)

- `keepapart list` on macOS shows `claude:?` instead of `claude:keychain`: Claude Code keeps macOS logins in the Keychain, which keepapart never reads, so it can't know. It now says how to check.
- Shared items that are copies (Windows without symlink rights) keep their modified time when copied, so newest-wins sync can't let an older edit win on Linux and macOS.
- This repo is now its own Scoop bucket (`bucket/`) and Homebrew tap (`Formula/`). Each release updates both, then CI installs through them on fresh Windows, macOS and Linux machines.
- First release on crates.io (`cargo install keepapart`) and winget.

## 0.1.0 (2026-11-12)

First release: isolated Claude Code and Codex profiles (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`), per-folder `.keepapart` files, shared or per-profile skills, agents, commands and settings, MCP sharing through `claude mcp add-json`, shell hooks for bash, zsh, fish and PowerShell, `doctor`, `export-config`, and the picker you get with no arguments. Tested live on Windows 11 with Claude Code and Codex; CI on Windows, macOS and Linux.
