//! gitree — native git worktree workflow tool.
//!
//! See `gitree --help` for usage.

mod clean;
mod cli;
mod completions;
mod doctor;
mod env;
mod error;
mod foreach;
mod format;
mod git;
mod init;
mod migrate;
mod pull;
mod repo;
mod shared;
mod status;
mod switch;
mod types;
mod worktree;

use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;

use cli::{Cli, Commands};
use init::InitOptions;
use migrate::MigrateOptions;
use pull::PullOptions;

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.verbose {
        git::set_verbose(true);
    }

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            if let Some(source) = e
                .source()
                .and_then(|s| s.downcast_ref::<error::GitreeError>())
                && let Some(hint) = hint_for(source)
            {
                eprintln!("hint: {hint}");
            }
            e.source()
                .and_then(|s| s.downcast_ref::<error::GitreeError>())
                .map_or(ExitCode::from(1), error::GitreeError::exit_code)
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        // Commands that don't need a wrapper.
        Commands::Init(args) => {
            init::run(InitOptions {
                url: args.url,
                name: args.name,
            })
            .context("failed to initialise gitree repository")?;
        }
        Commands::Migrate(args) => {
            migrate::run(MigrateOptions {
                yes: args.yes,
                force: args.force,
            })
            .context("failed to migrate repository")?;
        }
        Commands::Completion(args) => {
            completions::run_completion(args.shell, Cli::command())?;
        }
        Commands::Mangen(args) => {
            completions::run_mangen(&args.dir, Cli::command())?;
        }
        Commands::Complete { context } => {
            completions::run_complete_branches(context.as_deref())?;
        }
        Commands::Env(args) => {
            env::run(&args.shell, &args.alias)?;
        }

        // Commands that need a wrapper.
        Commands::Add(args) => {
            let wrapper = repo::Wrapper::discover()?;
            worktree::run_add(
                &wrapper,
                worktree::AddOptions {
                    branch: args.branch,
                    new: args.new,
                    base: args.base,
                },
            )
            .context("failed to add worktree")?;
        }
        Commands::Remove(args) => {
            let wrapper = repo::Wrapper::discover()?;
            worktree::run_remove(
                &wrapper,
                worktree::RemoveOptions {
                    branch: args.branch,
                    delete_branch: args.delete_branch,
                    force: args.force,
                },
            )
            .context("failed to remove worktree")?;
        }
        Commands::List(args) => {
            let wrapper = repo::Wrapper::discover()?;
            worktree::run_list(
                &wrapper,
                worktree::ListOptions {
                    json: args.json,
                    color: args.color,
                    path: args.path,
                },
            )?;
        }
        Commands::Prune => {
            let wrapper = repo::Wrapper::discover()?;
            worktree::run_prune(&wrapper)?;
        }
        Commands::Switch(args) => {
            let wrapper = repo::Wrapper::discover()?;
            switch::run_switch(&wrapper, &args.branch)?;
        }
        Commands::Where(args) => {
            let wrapper = repo::Wrapper::discover()?;
            worktree::run_where(&wrapper, &args.branch)?;
        }
        Commands::Root => {
            let wrapper = repo::Wrapper::discover()?;
            switch::run_root(&wrapper);
        }
        Commands::Pull(args) => {
            let wrapper = repo::Wrapper::discover()?;
            pull::run(
                &wrapper,
                PullOptions {
                    branch: args.branch,
                    autostash: args.autostash,
                },
            )
            .context("failed to pull")?;
        }
        Commands::Foreach(args) => {
            let wrapper = repo::Wrapper::discover()?;
            foreach::run(
                &wrapper,
                foreach::ForeachOptions {
                    command: args.command,
                    parallel: args.parallel,
                    only: args.only,
                },
            )
            .context("foreach failed")?;
        }
        Commands::Status => {
            let wrapper = repo::Wrapper::discover()?;
            status::run(&wrapper)?;
        }
        Commands::Doctor => {
            let wrapper = repo::Wrapper::discover()?;
            doctor::run(&wrapper)?;
        }
        Commands::Clean(args) => {
            let wrapper = repo::Wrapper::discover()?;
            clean::run(&wrapper, clean::CleanOptions { force: args.force })
                .context("failed to clean")?;
        }
    }

    Ok(())
}

/// Returns a helpful hint for specific error types.
fn hint_for(err: &error::GitreeError) -> Option<String> {
    use error::{DirtyEscape, GitreeError};
    match err {
        GitreeError::NotAWrapper(_) => Some(
            "run `gitree init <url>` to create a new repository, \
             or `gitree migrate` to convert an existing clone"
                .into(),
        ),
        GitreeError::BranchNotFound(name) => Some(format!(
            "to create a new branch, run: gitree add {name} --new"
        )),
        GitreeError::WorktreeExists(name) => {
            Some(format!("to switch to it: eval \"$(gitree switch {name})\""))
        }
        GitreeError::DirtyWorktree {
            branch,
            path,
            escape,
            ..
        } => {
            let mut hint = String::new();
            if let Some(p) = path {
                if let Some(b) = branch {
                    hint.push_str(&format!("worktree '{b}' is at {} — ", p.display()));
                } else {
                    hint.push_str(&format!("worktree is at {} — ", p.display()));
                }
            }
            match escape {
                DirtyEscape::Autostash => {
                    hint.push_str("commit or stash before pulling, or use --autostash to stash and pull automatically");
                }
                DirtyEscape::Force => {
                    hint.push_str(
                        "commit or stash before migrating, or use --force to proceed anyway",
                    );
                }
            }
            Some(hint)
        }
        GitreeError::GitNotFound => Some("install git and ensure it is on your PATH".into()),
        _ => None,
    }
}
