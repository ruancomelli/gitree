//! Command-line interface definition using `clap` derive.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;

use crate::format::{ColorPolicy, PathPolicy};

/// gitree — native git worktree workflow tool
///
/// A thin wrapper around `git worktree` that implements the bare-clone +
/// worktree layout described in the practical guide.  gitree is NOT a
/// replacement for git — you still use `git add`, `git commit`, etc.
#[derive(Parser, Debug)]
#[command(
    name = "gitree",
    version,
    about = "Native git worktree workflow tool",
    long_about = "gitree — native git worktree workflow tool\n\
        \n\
        A thin wrapper around `git worktree` that implements the bare-clone\n\
        + worktree layout.  gitree is NOT a replacement for git — you still\n\
        use `git add`, `git commit`, etc.\n\
        \n\
        The wrapper directory layout:\n\
        \n  \
        my-project/            ← wrapper (you never work here directly)\n  \
        ├── .bare/             ← git database (shared by all worktrees)\n  \
        ├── .git               ← file pointing to .bare\n  \
        ├── .shared/           ← gitignored files symlinked into worktrees\n  \
        └── <branch>/          ← one directory per worktree\n\
        \n\
        See `gitree init --help` and `gitree migrate --help` for setup."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Verbose output (shows underlying git commands).
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialise a new gitree wrapper from a remote URL.
    Init(InitArgs),

    /// Migrate an existing regular clone to a gitree wrapper layout.
    Migrate(MigrateArgs),

    /// Add a worktree for a branch.
    ///
    /// Tab-completes existing branch names.  By default, checks out an
    /// existing branch.  Use `--new` to create a new branch.
    #[command(alias = "a")]
    Add(AddArgs),

    /// Remove a worktree.
    #[command(alias = "rm")]
    Remove(RemoveArgs),

    /// List all worktrees.
    #[command(alias = "ls")]
    List(ListArgs),

    /// Prune stale worktree references.
    Prune,

    /// Print a `cd` command for switching to a worktree.
    ///
    /// Usage: `eval "$(gitree switch <branch>)"` or use `gitree env` for
    /// a `gtsw` shell function that changes directory natively.
    #[command(alias = "sw")]
    Switch(SwitchArgs),

    /// Print the path of a worktree.
    Where(WhereArgs),

    /// Print the wrapper root directory.
    Root,

    /// Fetch and fast-forward the main worktree.
    #[command(alias = "pl")]
    Pull(PullArgs),

    /// Run a command in every worktree.
    #[command(alias = "fe")]
    Foreach(ForeachArgs),

    /// Show status overview of all worktrees.
    #[command(alias = "st")]
    Status,

    /// Health check for the gitree wrapper.
    #[command(alias = "doc")]
    Doctor,

    /// Remove stale worktrees and delete branches gone from remote.
    Clean(CleanArgs),

    /// Generate shell integration script (defines `gtr` and `gtrsw`).
    ///
    /// Usage: `eval "$(gitree env bash)"`
    ///
    /// Pass `--alias <name>` to customise the function name (default: `gtr`).
    Env(EnvArgs),

    /// Generate shell completion script.
    Completion(CompletionArgs),

    /// Generate manpage(s).
    #[command(name = "__mangen", hide = true)]
    Mangen(MangenArgs),

    /// Internal: dynamic branch completion.
    ///
    /// `gitree __complete <context>` prints branch names suitable for TAB
    /// completion, scoped to the given subcommand context:
    ///
    /// - `add` (default): branches not already checked out as a worktree.
    /// - `remove` / `switch` / `where`: branches that are worktrees.
    /// - `base`: all local + remote branches (for `add --base`).
    #[command(name = "__complete", hide = true)]
    Complete {
        /// Subcommand context: `add`, `remove`, `switch`, `where`, or `base`.
        context: Option<String>,
    },
}

/// Arguments for `gitree init`.
#[derive(Args, Debug)]
pub struct InitArgs {
    /// Remote URL to clone from.
    pub url: String,

    /// Wrapper directory name (defaults to repo name from URL).
    #[arg(long)]
    pub name: Option<String>,

    /// Skip confirmation prompts.
    #[arg(short, long)]
    pub yes: bool,
}

/// Arguments for `gitree migrate`.
#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// Skip confirmation prompt.
    #[arg(short, long)]
    pub yes: bool,

    /// Allow migration even with warnings (untracked files, local-only
    /// branches).
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `gitree add`.
#[derive(Args, Debug)]
pub struct AddArgs {
    /// Branch name to create a worktree for.
    pub branch: String,

    /// Create a new branch instead of checking out an existing one.
    #[arg(short, long)]
    pub new: bool,

    /// Base ref when creating a new branch (default: HEAD of current worktree,
    /// or main/master).
    #[arg(long)]
    pub base: Option<String>,
}

/// Arguments for `gitree remove`.
#[derive(Args, Debug)]
pub struct RemoveArgs {
    /// Branch name whose worktree to remove.
    pub branch: String,

    /// Also delete the local branch.
    #[arg(long)]
    pub delete_branch: bool,

    /// Force removal even if the worktree is dirty.
    #[arg(short, long)]
    pub force: bool,
}

/// Arguments for `gitree list`.
#[derive(Args, Debug)]
pub struct ListArgs {
    /// Output as JSON.
    #[arg(long)]
    pub json: bool,

    /// Color output: always, never, or auto.
    #[arg(long, value_enum, default_value = "auto")]
    pub color: ColorPolicy,

    /// How to display worktree paths: relative (to CWD), absolute, or
    /// abbreviated (home directory shown as `~`).
    #[arg(long, value_enum, default_value = "relative")]
    pub path: PathPolicy,
}

/// Arguments for `gitree switch`.
#[derive(Args, Debug)]
pub struct SwitchArgs {
    /// Branch name to switch to.
    pub branch: String,
}

/// Arguments for `gitree where`.
#[derive(Args, Debug)]
pub struct WhereArgs {
    /// Branch name.
    pub branch: String,
}

/// Arguments for `gitree pull`.
#[derive(Args, Debug)]
pub struct PullArgs {
    /// Override the branch to fast-forward (default: main, fallback master).
    #[arg(long)]
    pub branch: Option<String>,
}

/// Arguments for `gitree foreach`.
#[derive(Args, Debug)]
pub struct ForeachArgs {
    /// Command to run (passed to `sh -c`).
    pub command: String,

    /// Run in parallel using threads.
    #[arg(long)]
    pub parallel: bool,

    /// Filter worktrees by branch glob pattern (e.g. `feature/*`).
    #[arg(long)]
    pub only: Option<String>,
}

/// Arguments for `gitree clean`.
#[derive(Args, Debug)]
pub struct CleanArgs {
    /// Delete stale branches without prompting.
    #[arg(short, long)]
    pub force: bool,
}

/// Arguments for `gitree env`.
#[derive(Args, Debug)]
pub struct EnvArgs {
    /// Shell to generate integration for.
    pub shell: String,

    /// Name for the shell function (default: `gtr`).
    #[arg(long, default_value = "gtr")]
    pub alias: String,
}

/// Arguments for `gitree completion`.
#[derive(Args, Debug)]
pub struct CompletionArgs {
    /// Shell to generate completions for.
    pub shell: Shell,
}

/// Arguments for `gitree __mangen`.
#[derive(Args, Debug)]
pub struct MangenArgs {
    /// Directory to write manpage(s) into.
    pub dir: PathBuf,
}

impl Cli {
    /// Returns a [`clap::Command`] for use in completion/manpage generation.
    #[must_use]
    pub fn command() -> clap::Command {
        <Self as clap::CommandFactory>::command().bin_name("gitree")
    }
}
