//! Optional shell hooks: plain `claude` / `codex` pick up the profile for the
//! folder you're in. The hook runs `keepapart env --shell <sh>` before each
//! prompt and evaluates its output, which only ever sets or unsets
//! CLAUDE_CONFIG_DIR, CODEX_HOME and KEEPAPART_ACTIVE.

use crate::profile::Tool;
use clap::ValueEnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    #[value(alias = "pwsh")]
    Powershell,
}

pub const ACTIVE_VAR: &str = "KEEPAPART_ACTIVE";

/// Quote a value for the given shell so any path (spaces, quotes, `$`) survives.
pub fn quote(shell: Shell, s: &str) -> String {
    match shell {
        Shell::Bash | Shell::Zsh => format!("'{}'", s.replace('\'', r"'\''")),
        Shell::Fish => format!("'{}'", s.replace('\\', r"\\").replace('\'', r"\'")),
        Shell::Powershell => format!("'{}'", s.replace('\'', "''")),
    }
}

fn set(shell: Shell, k: &str, v: &str) -> String {
    match shell {
        Shell::Bash | Shell::Zsh => format!("export {k}={}", quote(shell, v)),
        Shell::Fish => format!("set -gx {k} {}", quote(shell, v)),
        Shell::Powershell => format!("$env:{k} = {}", quote(shell, v)),
    }
}

fn unset(shell: Shell, k: &str) -> String {
    match shell {
        Shell::Bash | Shell::Zsh => format!("unset {k}"),
        Shell::Fish => format!("set -e {k}"),
        Shell::Powershell => format!("Remove-Item Env:{k} -ErrorAction SilentlyContinue"),
    }
}

/// What the environment should look like.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desired {
    pub profile: String,
    /// (tool, dir) for each tool the profile uses.
    pub dirs: Vec<(Tool, String)>,
}

/// Lines to evaluate. `current` reads the shell's current environment.
/// Nothing is printed when the environment already matches, and variables are
/// only unset if keepapart set them (KEEPAPART_ACTIVE is present).
pub fn env_script(shell: Shell, desired: Option<&Desired>, current: &dyn Fn(&str) -> Option<String>) -> String {
    let managed = current(ACTIVE_VAR).is_some();
    let mut lines = Vec::new();
    match desired {
        Some(d) => {
            for t in Tool::ALL {
                let want = d.dirs.iter().find(|(tt, _)| *tt == t).map(|(_, p)| p.as_str());
                let have = current(t.env_var());
                match want {
                    Some(w) if have.as_deref() != Some(w) => lines.push(set(shell, t.env_var(), w)),
                    None if managed && have.is_some() => lines.push(unset(shell, t.env_var())),
                    _ => {}
                }
            }
            if current(ACTIVE_VAR).as_deref() != Some(d.profile.as_str()) {
                lines.push(set(shell, ACTIVE_VAR, &d.profile));
            }
        }
        None if managed => {
            for t in Tool::ALL {
                if current(t.env_var()).is_some() {
                    lines.push(unset(shell, t.env_var()));
                }
            }
            lines.push(unset(shell, ACTIVE_VAR));
        }
        None => {}
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

pub fn hook(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => {
            r#"# keepapart: apply the folder's profile before each prompt
_keepapart_hook() {
  local _kp_status=$?
  eval "$(command keepapart env --shell bash 2>/dev/null)"
  return $_kp_status
}
case ";${PROMPT_COMMAND:-};" in
  *";_keepapart_hook;"*) ;;
  *) PROMPT_COMMAND="_keepapart_hook${PROMPT_COMMAND:+;$PROMPT_COMMAND}" ;;
esac
"#
        }
        Shell::Zsh => {
            r#"# keepapart: apply the folder's profile before each prompt
_keepapart_hook() { eval "$(command keepapart env --shell zsh 2>/dev/null)"; }
typeset -ag precmd_functions
if (( ! ${precmd_functions[(I)_keepapart_hook]} )); then
  precmd_functions=(_keepapart_hook $precmd_functions)
fi
"#
        }
        Shell::Fish => {
            r#"# keepapart: apply the folder's profile before each prompt
function __keepapart_hook --on-event fish_prompt
    command keepapart env --shell fish 2>/dev/null | source
end
"#
        }
        Shell::Powershell => {
            r#"# keepapart: apply the folder's profile before each prompt
if (-not $global:__keepapartHooked) {
    $global:__keepapartHooked = $true
    $global:__keepapartPrompt = $function:prompt
    function global:prompt {
        $kp = (& keepapart env --shell powershell 2>$null) -join "`n"
        if ($kp) { Invoke-Expression $kp }
        & $global:__keepapartPrompt
    }
}
"#
        }
    }
}

pub fn install_hint(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => r#"Add to ~/.bashrc:   eval "$(keepapart hook bash)""#,
        Shell::Zsh => r#"Add to ~/.zshrc:    eval "$(keepapart hook zsh)""#,
        Shell::Fish => "Add to ~/.config/fish/config.fish:   keepapart hook fish | source",
        Shell::Powershell => {
            r#"Add to your PowerShell profile (notepad $PROFILE):   Invoke-Expression ((& keepapart hook powershell) -join "`n")"#
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn quoting_survives_hostile_paths() {
        let p = r"C:\Users\O'Brien\it's $HOME\x";
        assert_eq!(quote(Shell::Bash, p), r"'C:\Users\O'\''Brien\it'\''s $HOME\x'");
        assert_eq!(quote(Shell::Powershell, p), r"'C:\Users\O''Brien\it''s $HOME\x'");
        assert_eq!(quote(Shell::Fish, p), r"'C:\\Users\\O\'Brien\\it\'s $HOME\\x'");
    }

    #[test]
    fn sets_both_dirs_for_a_both_profile() {
        let d = Desired {
            profile: "work".into(),
            dirs: vec![(Tool::Claude, "/k/work/claude".into()), (Tool::Codex, "/k/work/codex".into())],
        };
        let s = env_script(Shell::Bash, Some(&d), &env(&[]));
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/k/work/claude'"));
        assert!(s.contains("export CODEX_HOME='/k/work/codex'"));
        assert!(s.contains("export KEEPAPART_ACTIVE='work'"));
    }

    #[test]
    fn quiet_when_already_applied() {
        let d = Desired { profile: "work".into(), dirs: vec![(Tool::Claude, "/k/w/claude".into())] };
        let cur = env(&[("CLAUDE_CONFIG_DIR", "/k/w/claude"), ("KEEPAPART_ACTIVE", "work")]);
        assert_eq!(env_script(Shell::Zsh, Some(&d), &cur), "");
    }

    #[test]
    fn switching_to_claude_only_profile_drops_codex_home() {
        let d = Desired { profile: "p".into(), dirs: vec![(Tool::Claude, "/k/p/claude".into())] };
        let cur = env(&[("CODEX_HOME", "/k/w/codex"), ("CLAUDE_CONFIG_DIR", "/k/w/claude"), ("KEEPAPART_ACTIVE", "w")]);
        let s = env_script(Shell::Powershell, Some(&d), &cur);
        assert!(s.contains("$env:CLAUDE_CONFIG_DIR = '/k/p/claude'"));
        assert!(s.contains("Remove-Item Env:CODEX_HOME"));
        assert!(s.contains("$env:KEEPAPART_ACTIVE = 'p'"));
    }

    #[test]
    fn leaving_a_profile_folder_unsets_only_what_we_set() {
        let ours = env(&[("CLAUDE_CONFIG_DIR", "/k/w/claude"), ("KEEPAPART_ACTIVE", "w")]);
        let s = env_script(Shell::Fish, None, &ours);
        assert!(s.contains("set -e CLAUDE_CONFIG_DIR"));
        assert!(s.contains("set -e KEEPAPART_ACTIVE"));
        assert!(!s.contains("CODEX_HOME"));
        // A user's own CLAUDE_CONFIG_DIR (no KEEPAPART_ACTIVE) is left alone.
        let theirs = env(&[("CLAUDE_CONFIG_DIR", "/mine")]);
        assert_eq!(env_script(Shell::Bash, None, &theirs), "");
    }

    #[test]
    fn hooks_call_env_for_their_own_shell() {
        for sh in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::Powershell] {
            let name = sh.to_possible_value().unwrap().get_name().to_string();
            assert!(hook(sh).contains(&format!("env --shell {name}")), "{name}");
        }
    }
}
