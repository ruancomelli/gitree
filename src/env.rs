//! `gitree env <shell>` — shell integration script generation.

use std::io::Write;

use crate::error::{GitreeError, Result};

/// Generates and prints shell integration script.
///
/// Defines `<alias>()` (dispatcher) and `<alias>sw` (switch with native `cd`),
/// and wires up shell completions so that `<alias>` delegates to the
/// `gitree` completion function installed via `gitree completion <shell>`.
///
/// # Errors
///
/// Returns an error if the shell is not supported or writing fails.
pub fn run(shell: &str, alias: &str) -> Result<()> {
    validate_alias(alias)?;
    let script = match shell {
        "bash" => bash_script(alias),
        "zsh" => zsh_script(alias),
        "fish" => fish_script(alias),
        "posix" | "sh" => posix_script(alias),
        _ => {
            return Err(GitreeError::Other(format!(
                "unsupported shell '{shell}'. Supported: bash, zsh, fish, posix"
            )));
        }
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    out.write_all(script.as_bytes())?;
    Ok(())
}

/// Validates that an alias name is a safe shell identifier.
///
/// Allows alphanumeric and underscore characters, starting with a letter
/// or underscore. This prevents injection of arbitrary shell commands
/// through `--alias`.
fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty() {
        return Err(GitreeError::Other("alias name must not be empty".into()));
    }
    let mut chars = alias.chars();
    let first = chars.next().expect("checked non-empty");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(GitreeError::Other(format!(
            "alias name must start with a letter or underscore: '{alias}'"
        )));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(GitreeError::Other(format!(
            "alias name may only contain letters, digits, or underscores: '{alias}'"
        )));
    }
    Ok(())
}

/// Builds the switch alias name from the base alias (e.g. `gtr` -> `gtrsw`).
fn switch_alias(alias: &str) -> String {
    format!("{alias}sw")
}

/// Shared function + alias definitions for bash and zsh (identical syntax).
fn bash_zsh_core(alias: &str) -> String {
    let sw = switch_alias(alias);
    format!(
        r#"# {alias}() — dispatcher: `{alias} sw <branch>` does native cd, everything
# else passes through to gitree.
{alias}() {{
    if [[ "$1" == "sw" || "$1" == "switch" ]]; then
        shift
        local result
        result=$(gitree switch "$@") && eval "$result"
    else
        gitree "$@"
    fi
}}

# {sw} — switch to a worktree (changes directory natively)
alias {sw}='{alias} sw'
"#
    )
}

fn bash_script(alias: &str) -> String {
    let lazy = format!("_{alias}_complete");
    format!(
        r#"# gitree shell integration for bash
# Usage: eval "$(gitree env bash)"

{core}
# Completions: {alias} delegates to the gitree bash completion function
# (installed via `gitree completion bash`).  The completion file is loaded
# lazily because it may not be sourced yet when {alias} is first completed.
{lazy}() {{
    if ! declare -F _gitree >/dev/null 2>&1; then
        local f
        for f in \
            "${{XDG_DATA_HOME:-$HOME/.local/share}}/bash-completion/completions/gitree" \
            "/usr/share/bash-completion/completions/gitree" \
            "/etc/bash_completion.d/gitree"; do
            [[ -f "$f" ]] && source "$f" && break
        done
    fi
    if declare -F _gitree >/dev/null 2>&1; then
        _gitree "$@"
    fi
}}
complete -F {lazy} {alias}
"#,
        core = bash_zsh_core(alias)
    )
}

fn zsh_script(alias: &str) -> String {
    format!(
        r#"# gitree shell integration for zsh
# Usage: eval "$(gitree env zsh)"

{core}
# Completions: {alias} delegates to the gitree zsh completion function
# (installed via `gitree completion zsh`).  `compdef` is available after
# `compinit`; guard in case this script is sourced before compinit runs.
if command -v compdef >/dev/null 2>&1; then
    compdef _gitree {alias}
fi
"#,
        core = bash_zsh_core(alias)
    )
}

fn fish_script(alias: &str) -> String {
    let sw = switch_alias(alias);
    format!(
        r#"# gitree shell integration for fish
# Usage: gitree env fish | source

# {alias}() — dispatcher: `{alias} sw <branch>` does native cd, everything
# else passes through to gitree.
function {alias}
    switch $argv[1]
        case sw switch
            gitree switch $argv[2..] | source
        case '*'
            gitree $argv
    end
end

# {sw} — switch to a worktree (changes directory natively).
# Defined as a function (not `alias {sw}="{alias} sw"`) so that its
# `--wraps` is `gitree switch`, giving branch-name TAB completion.  An alias
# would set `--wraps` to `{alias} sw`, and clap's fish completions only match
# the canonical subcommand name `switch`, so branch completion would never
# fire.
function {sw} --wraps='gitree switch' --description='switch to a worktree'
    {alias} sw $argv
end

# Completion: {alias} delegates to gitree.  The gitree.fish completion rules
# (installed via `gitree completion fish`) are applied to {alias} via fish's
# --wraps mechanism.
complete --command {alias} --wraps gitree
"#
    )
}

fn posix_script(alias: &str) -> String {
    let sw = switch_alias(alias);
    format!(
        r#"# gitree shell integration for POSIX shells
# Usage: eval "$(gitree env posix)"

# {alias}() — dispatcher: `{alias} sw <branch>` does native cd, everything
# else passes through to gitree.
{alias}() {{
    if [ "$1" = "sw" ] || [ "$1" = "switch" ]; then
        shift
        result=$(gitree switch "$@") && eval "$result"
    else
        gitree "$@"
    fi
}}

# {sw} — switch to a worktree (changes directory natively)
alias {sw}='{alias} sw'
"#
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_script_default_alias() {
        let script = bash_script("gtr");
        assert!(script.contains("gtr()"));
        assert!(script.contains("gtrsw"));
        assert!(script.contains("gitree switch"));
        assert!(script.contains("complete -F _gtr_complete gtr"));
        assert!(script.contains("_gtr_complete"));
    }

    #[test]
    fn bash_script_custom_alias() {
        let script = bash_script("mygt");
        assert!(script.contains("mygt()"));
        assert!(script.contains("mygtsw"));
        assert!(script.contains("complete -F _mygt_complete mygt"));
        assert!(!script.contains("gtr"));
    }

    #[test]
    fn zsh_script_default_alias() {
        let script = zsh_script("gtr");
        assert!(script.contains("gtr()"));
        assert!(script.contains("gtrsw"));
        assert!(script.contains("compdef _gitree gtr"));
    }

    #[test]
    fn zsh_script_custom_alias() {
        let script = zsh_script("wt");
        assert!(script.contains("wt()"));
        assert!(script.contains("wtsw"));
        assert!(script.contains("compdef _gitree wt"));
    }

    #[test]
    fn fish_script_default_alias() {
        let script = fish_script("gtr");
        assert!(script.contains("function gtr\n"));
        assert!(script.contains("function gtrsw --wraps='gitree switch'"));
        assert!(script.contains("gtr sw $argv"));
        assert!(script.contains("complete --command gtr --wraps gitree"));
    }

    #[test]
    fn fish_script_custom_alias() {
        let script = fish_script("wt");
        assert!(script.contains("function wt\n"));
        assert!(script.contains("function wtsw --wraps='gitree switch'"));
        assert!(script.contains("wt sw $argv"));
        assert!(script.contains("complete --command wt --wraps gitree"));
    }

    #[test]
    fn posix_script_default_alias() {
        let script = posix_script("gtr");
        assert!(script.contains("gtr()"));
        assert!(script.contains("gtrsw"));
    }

    #[test]
    fn posix_script_custom_alias() {
        let script = posix_script("gw");
        assert!(script.contains("gw()"));
        assert!(script.contains("gwsw"));
    }

    #[test]
    fn validate_alias_accepts_valid_names() {
        assert!(validate_alias("gtr").is_ok());
        assert!(validate_alias("_gt").is_ok());
        assert!(validate_alias("my_alias_2").is_ok());
    }

    #[test]
    fn validate_alias_rejects_empty() {
        assert!(validate_alias("").is_err());
    }

    #[test]
    fn validate_alias_rejects_invalid_start() {
        assert!(validate_alias("1gt").is_err());
        assert!(validate_alias("-gt").is_err());
    }

    #[test]
    fn validate_alias_rejects_special_chars() {
        assert!(validate_alias("gt-r").is_err());
        assert!(validate_alias("gt;rm").is_err());
        assert!(validate_alias("gt rm").is_err());
    }
}
