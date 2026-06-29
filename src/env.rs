//! `gitree env <shell>` — shell integration script generation.

use std::io::Write;

use crate::error::{GitreeError, Result};

/// Generates and prints shell integration script.
///
/// Defines `gt()` (dispatcher) and `gtsw` (switch with native `cd`).
///
/// # Errors
///
/// Returns an error if the shell is not supported or writing fails.
pub fn run(shell: &str) -> Result<()> {
    let script = match shell {
        "bash" | "zsh" => bash_script(),
        "fish" => fish_script(),
        "posix" | "sh" => posix_script(),
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

fn bash_script() -> String {
    r#"# gitree shell integration for bash/zsh
# Usage: eval "$(gitree env bash)"

# gt() — dispatcher: `gt sw <branch>` does native cd, everything else passes
# through to gitree.
gt() {
    if [[ "$1" == "sw" || "$1" == "switch" ]]; then
        shift
        local result
        result=$(gitree switch "$@") && eval "$result"
    else
        gitree "$@"
    fi
}

# gtsw — switch to a worktree (changes directory natively)
alias gtsw='gt sw'
"#
    .to_string()
}

fn fish_script() -> String {
    r#"# gitree shell integration for fish
# Usage: gitree env fish | source

# gt() — dispatcher: `gt sw <branch>` does native cd, everything else passes
# through to gitree.
function gt
    switch $argv[1]
        case sw switch
            gitree switch $argv[2..] | source
        case '*'
            gitree $argv
    end
end

# gtsw — switch to a worktree (changes directory natively)
alias gtsw='gt sw'
"#
    .to_string()
}

fn posix_script() -> String {
    r#"# gitree shell integration for POSIX shells
# Usage: eval "$(gitree env posix)"

# gt() — dispatcher: `gt sw <branch>` does native cd, everything else passes
# through to gitree.
gt() {
    if [ "$1" = "sw" ] || [ "$1" = "switch" ]; then
        shift
        result=$(gitree switch "$@") && eval "$result"
    else
        gitree "$@"
    fi
}

# gtsw — switch to a worktree (changes directory natively)
alias gtsw='gt sw'
"#
    .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_script_contains_gt_function() {
        let script = bash_script();
        assert!(script.contains("gt()"));
        assert!(script.contains("gtsw"));
        assert!(script.contains("gitree switch"));
    }

    #[test]
    fn fish_script_contains_gt_function() {
        let script = fish_script();
        assert!(script.contains("function gt"));
        assert!(script.contains("gtsw"));
    }

    #[test]
    fn posix_script_contains_gt_function() {
        let script = posix_script();
        assert!(script.contains("gt()"));
        assert!(script.contains("gtsw"));
    }
}
