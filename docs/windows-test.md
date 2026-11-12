# Live test on Windows 11

Run on 2026-11-12 on a real Windows 11 Home (10.0.26200) machine with **Claude Code 2.1.322** (native installer) and **Codex CLI 0.167.2** (npm). The account holder did the browser logins; keepapart never saw them. The email is masked below, and `~` is `C:\Users\<user>`.

The machine already had a normal Claude Code login in `~/.claude` and a normal Codex login in `~/.codex`. Neither was touched.

## 1. Two profiles, two logins

```text
> keepapart add work --tool claude --no-login
> keepapart add personal --tool claude --no-login
> keepapart login work          # runs `claude auth login` with CLAUDE_CONFIG_DIR=~/.keepapart/profiles/work/claude
Login successful.
> keepapart login personal
Login successful.

> keepapart list
  NAME      TOOLS         LOGIN FILE                LAST USED
  personal  claude        claude:yes                never
  work      claude        claude:yes                2m ago

> keepapart run work -- auth status --text
Login method: Claude Pro account
Email: m***@gmail.com
> keepapart run personal -- auth status --text
Login method: Claude Pro account
Email: m***@gmail.com
```

Both profiles used the same subscription account (it's the only paid one available), so the logout test in step 3 is what proves they're separate.

Login files, metadata only (keepapart never opens them):

```text
~/.keepapart/profiles/work/claude/.credentials.json      size=554  mtime=11:40:30  inode=8725724279568446
~/.keepapart/profiles/personal/claude/.credentials.json  size=554  mtime=11:41:12  inode=9570149209710172
~/.claude/.credentials.json                              size=554  mtime=10:57:43  inode=10133099163121529   (before the test: 10:57:43)
```

Three separate regular files, none a link. Claude Code's `.claude.json` was also created inside each profile folder (`profiles/<name>/claude/.claude.json`), so it moves with `CLAUDE_CONFIG_DIR` too.

## 2. Side by side

Both profiles started at the same moment, each with one short prompt:

```text
[work]     start=11:43:10.242 end=11:43:21.241 reply=hello from the work profile
[personal] start=11:43:10.283 end=11:43:21.212 reply=hello from the personal profile
```

Each process wrote its session transcript into its own profile only:

```text
profiles/work/claude/projects/<folder>/d69aad24-….jsonl
profiles/personal/claude/projects/<folder>/17e758b0-….jsonl
default ~/.claude/projects transcripts: before=86 after=86
```

## 3. Logging one out doesn't touch the other

```text
> keepapart run personal -- auth logout
Successfully logged out from your Anthropic account.

> keepapart run work -- auth status --text
Login method: Claude Pro account
Email: m***@gmail.com
> keepapart run personal -- auth status --text
Not logged in. Run claude auth login to authenticate.

> keepapart run work -- -p "Reply with exactly: work still signed in"
work still signed in

~/.keepapart/profiles/work/claude/.credentials.json   size=554  mtime=11:40:30   (unchanged)
~/.keepapart/profiles/personal/claude/.credentials.json   (gone: removed by Claude Code's own logout)
~/.claude/.credentials.json                           size=554  mtime=10:57:43   (unchanged)
```

## 4. Codex

```text
> keepapart add work --tool codex --no-login
Added codex to profile "work".
> codex login status                                   # the normal ~/.codex
Logged in using ChatGPT
> keepapart run work --tool codex -- login status      # CODEX_HOME=~/.keepapart/profiles/work/codex
Not logged in
```

The machine's existing Codex login is not visible inside the profile. A second Codex login wasn't done in this run. The Codex isolation path is also covered by the end-to-end tests.

## 5. doctor

```text
Official CLIs
  ok    claude: ~\.local\bin\claude.exe (2.1.322 (Claude Code))
  ok    codex: ~\AppData\Roaming\npm\codex.cmd (codex-cli 0.167.2)
Links
  ok    symlinks allowed: shared items stay live in every profile
Profiles
  [personal]
  info  claude: own folder, not logged in: keepapart login personal
  ok    claude:skills shared (symlink)  …
  [work]
  ok    claude: own folder, login file present
  ok    claude:skills shared (symlink)  …
```

This machine has symlink rights (Developer Mode). The junction and copy fallback for accounts without them is covered by `tests/cli.rs` (`without_symlink_rights_…`), which runs on Windows in CI.

## Found during the test, fixed

- The Claude Code native installer puts `claude.exe` in `~\.local\bin` and doesn't add that folder to PATH. keepapart now spots this and says exactly which folder to add, instead of just "not on PATH".
- `keepapart login` now uses the official `claude auth login` command instead of opening a full session.
