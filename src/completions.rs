//! `gitree completion`, `gitree __complete`, `gitree __mangen`.
//!
//! Subcommands whose name starts with a double underscore are hidden internal
//! helpers that generated completion scripts invoke; they are not part of the
//! user-facing CLI surface.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use clap::Command;
use clap_complete::{Shell, generate};
use clap_mangen::Man;

use crate::error::Result;
use crate::git::Git;
use crate::repo::Wrapper;

/// Generates and prints shell completion script.
///
/// For `bash`, `zsh`, and `fish`, the static `clap_complete` output is
/// post-processed to wire the `branch` positional of `add`, `remove`,
/// `switch`, and `where` (and the `--base` option of `add`) to dynamic
/// completion via `gitree __complete <context>`.
///
/// # Errors
///
/// Returns an error if writing fails.
pub fn run_completion(shell: Shell, mut cmd: Command) -> Result<()> {
    cmd.build();

    let mut buffer: Vec<u8> = Vec::new();
    generate(shell, &mut cmd, "gitree", &mut buffer);

    match shell {
        Shell::Bash => append_bash_overrides(&mut buffer),
        Shell::Zsh => append_zsh_overrides(&mut buffer)?,
        Shell::Fish => append_fish_overrides(&mut buffer),
        _ => {}
    }

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    out.write_all(&buffer)?;
    Ok(())
}

/// Dynamic completion: prints branch names for `gitree __complete <context>`.
///
/// `context` selects the candidate set:
///
/// - `"add"` or `None` (default): branches not already checked out as a
///   worktree.
/// - `"remove"` / `"switch"` / `"where"`: branches that are worktrees.
/// - `"base"`: all local + remote branches (deduplicated).
///
/// Silently prints nothing if the CWD is not inside a gitree wrapper, so
/// shell completion never emits errors to the terminal.
///
/// # Errors
///
/// Returns an error if writing fails.
pub fn run_complete_branches(context: Option<&str>) -> Result<()> {
    let wrapper = match Wrapper::discover() {
        Ok(w) => w,
        Err(_) => return Ok(()),
    };
    let git = wrapper.git();
    let branches = match context {
        Some("remove") | Some("switch") | Some("where") => worktree_branches(&git),
        Some("base") => all_branches(&git),
        Some("add") | Some(_) | None => add_branches(&git),
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for b in &branches {
        writeln!(out, "{b}")?;
    }
    Ok(())
}

/// Branches not currently checked out as a worktree (the `add` set).
fn add_branches(git: &Git) -> Vec<String> {
    let branches = git.branches().unwrap_or_default();
    let active: HashSet<String> = git
        .worktree_list()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|wt| wt.branch)
        .collect();
    let local_set: HashSet<&str> = branches.local.iter().map(String::as_str).collect();

    let mut result = Vec::new();
    for b in &branches.local {
        if !active.contains(b) {
            result.push(b.clone());
        }
    }
    for b in &branches.remote {
        if !active.contains(b) && !local_set.contains(b.as_str()) {
            result.push(b.clone());
        }
    }
    result
}

/// Branches that currently have a worktree (the `remove`/`switch`/`where`
/// set).
fn worktree_branches(git: &Git) -> Vec<String> {
    git.worktree_list()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|wt| wt.branch)
        .collect()
}

/// All local + remote branches, deduplicated (the `base` set).
fn all_branches(git: &Git) -> Vec<String> {
    let branches = git.branches().unwrap_or_default();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut result = Vec::new();
    for b in branches.local.iter().chain(branches.remote.iter()) {
        if seen.insert(b.as_str()) {
            result.push(b.clone());
        }
    }
    result
}

/// Appends a bash wrapper that intercepts branch positional completion for
/// `add`/`remove`/`switch`/`where` (and their aliases) and `--base` values,
/// delegating to `gitree __complete <context>`.
fn append_bash_overrides(buffer: &mut Vec<u8>) {
    let script = r#"

# gitree: dynamic branch completion (appended by `gitree completion bash`).
# Renames the clap-generated _gitree function and wraps it so that the
# branch positional of add/remove/switch/where and the --base option of add
# are completed dynamically via `gitree __complete <context>`.
if declare -F _gitree >/dev/null 2>&1; then
    eval "$(declare -f _gitree | sed '1s/^[A-Za-z_][A-Za-z0-9_]*/__gitree_clap_orig/')"
    _gitree() {
        local sub="${COMP_WORDS[1]:-}"
        local cur prev
        if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
            cur="$2"
        else
            cur="${COMP_WORDS[COMP_CWORD]}"
        fi
        prev="$3"

        # Normalize subcommand aliases so branch completion fires when the
        # user types `a`, `rm`, or `sw`.
        case "${sub}" in
            a) sub="add" ;;
            rm) sub="remove" ;;
            sw) sub="switch" ;;
        esac

        case "${sub}" in
            add)
                if [[ "${prev}" == "--base" ]]; then
                    COMPREPLY=( $(compgen -W "$(gitree __complete base 2>/dev/null)" -- "${cur}") )
                    return 0
                fi
                if [[ "${cur}" != -* ]]; then
                    local i has_new=0
                    for ((i=2; i<COMP_CWORD; i++)); do
                        case "${COMP_WORDS[i]}" in
                            --new|-n) has_new=1 ;;
                        esac
                    done
                    if [[ $has_new -eq 0 ]]; then
                        COMPREPLY=( $(compgen -W "$(gitree __complete add 2>/dev/null)" -- "${cur}") )
                        return 0
                    fi
                fi
                ;;
            remove|switch|where)
                if [[ "${cur}" != -* ]]; then
                    COMPREPLY=( $(compgen -W "$(gitree __complete "${sub}" 2>/dev/null)" -- "${cur}") )
                    return 0
                fi
                ;;
        esac
        __gitree_clap_orig "$@"
    }
fi
"#;
    buffer.extend_from_slice(script.as_bytes());
}

/// Appends zsh helper functions and rewrites the generated `_default` action
/// tags for branch positionals and `--base` so they call those helpers.
///
/// # Errors
///
/// Returns an error if the generated script is not valid UTF-8.
fn append_zsh_overrides(buffer: &mut Vec<u8>) -> Result<()> {
    let mut script = String::from_utf8(std::mem::take(buffer))
        .map_err(|e| crate::error::GitreeError::Other(e.to_string()))?;

    // Rewrite the `_default` action for each branch positional and for
    // `--base` to point at our custom helpers.  Each target substring is
    // unique within the generated script.
    script = script
        .replace(
            "Branch name to create a worktree for:_default",
            "Branch name to create a worktree for:_gitree_complete_add",
        )
        .replace("BASE:_default", "BASE:_gitree_complete_base")
        .replace(
            "Branch name whose worktree to remove:_default",
            "Branch name whose worktree to remove:_gitree_complete_remove",
        )
        .replace(
            "Branch name to switch to:_default",
            "Branch name to switch to:_gitree_complete_switch",
        )
        .replace("Branch name:_default", "Branch name:_gitree_complete_where");

    let helpers = r#"

# gitree: dynamic branch completion (appended by `gitree completion zsh`).
_gitree_complete_add() {
    if (( ${words[(I)--new]} || ${words[(I)-n]} )); then
        return 0
    fi
    local -a branches
    branches=("${(@f)$(gitree __complete add 2>/dev/null)}")
    compadd -- "${branches[@]}"
}
_gitree_complete_remove() {
    local -a branches
    branches=("${(@f)$(gitree __complete remove 2>/dev/null)}")
    compadd -- "${branches[@]}"
}
_gitree_complete_switch() {
    local -a branches
    branches=("${(@f)$(gitree __complete switch 2>/dev/null)}")
    compadd -- "${branches[@]}"
}
_gitree_complete_where() {
    local -a branches
    branches=("${(@f)$(gitree __complete where 2>/dev/null)}")
    compadd -- "${branches[@]}"
}
_gitree_complete_base() {
    local -a branches
    branches=("${(@f)$(gitree __complete base 2>/dev/null)}")
    compadd -- "${branches[@]}"
}
"#;
    script.push_str(helpers);
    buffer.extend_from_slice(script.as_bytes());
    Ok(())
}

/// Appends fish `complete` rules that offer branch names for the relevant
/// subcommands and base refs for `add --base`.  The guards match the
/// canonical subcommand names *and* their aliases, since the helper checks
/// the literal token the user typed.
fn append_fish_overrides(buffer: &mut Vec<u8>) {
    let script = r#"

# gitree: dynamic branch completion (appended by `gitree completion fish`).
# Without these rules fish falls back to file completion for branch
# positionals, so `gtr rm <prefix><TAB>` offers directory names.
complete -c gitree -n '__fish_gitree_using_subcommand add a; and not __fish_contains_opt -s n new' -f -a '(gitree __complete add)' -d 'Branch'
complete -c gitree -n '__fish_gitree_using_subcommand add a; and __fish_prev_arg_in --base' -f -a '(gitree __complete base)' -d 'Base ref'
complete -c gitree -n '__fish_gitree_using_subcommand remove rm' -f -a '(gitree __complete remove)' -d 'Branch'
complete -c gitree -n '__fish_gitree_using_subcommand switch sw' -f -a '(gitree __complete switch)' -d 'Branch'
complete -c gitree -n '__fish_gitree_using_subcommand where' -f -a '(gitree __complete where)' -d 'Branch'
"#;
    buffer.extend_from_slice(script.as_bytes());
}

/// Generates manpage(s) into a directory.
///
/// # Errors
///
/// Returns an error if writing fails.
pub fn run_mangen(dir: &Path, mut cmd: Command) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    cmd.build();
    let man = Man::new(cmd);
    let mut buffer: Vec<u8> = Vec::new();
    man.render(&mut buffer)?;

    let path = dir.join("gitree.1");
    let mut file = std::fs::File::create(&path)?;
    file.write_all(&buffer)?;
    eprintln!("Wrote {}", path.display());
    Ok(())
}
