# AGENTS.md

> **Living document.** This file is the single source of truth for coding
> agents working on gitree. Keep it up to date — see
> [Maintaining this file](#maintaining-this-file) below.

## Project overview

**gitree** is a native git worktree workflow tool written in Rust. It wraps
`git worktree` operations with sensible defaults, shell integration, and
`.shared/` symlink automation. It is **not** a replacement for git — users
still run `git add`, `git commit`, etc.

gitree implements the bare-clone + worktree layout:

```
my-project/              ← wrapper (never work here directly)
├── .bare/               ← git database (shared by all worktrees)
├── .git                 ← file: "gitdir: ./.bare"
├── .shared/             ← gitignored files symlinked into each worktree
└── <branch>/            ← one directory per worktree
```

## Tech stack

- **Language:** Rust, edition 2024
- **MSRV:** 1.96 (`rust-version` in `Cargo.toml`)
- **CLI framework:** `clap` v4 (derive)
- **Completions/manpages:** `clap_complete`, `clap_mangen`
- **Error handling:** `thiserror` (library-level typed errors) + `anyhow`
  (application-level context chaining in `main.rs`)
- **Serialization:** `serde` + `serde_json` (for `--json` output)
- **Glob matching:** `globset` (for `foreach --only`)
- **Relative path computation:** `pathdiff` (for `gitree list --path relative`)
- **No async.** All operations are synchronous.
- **No git crate.** gitree shells out to the real `git` binary via
  `std::process::Command`.

## Build and test commands

```sh
# Build
cargo build

# Build (release)
cargo build --release

# Run all tests (unit + integration)
cargo test

# Run only unit tests
cargo test --lib

# Run only integration tests
cargo test --test integration

# Format check
cargo fmt --check

# Format (apply)
cargo fmt

# Lint (must pass with zero warnings)
cargo clippy --all-targets -- -D warnings

# Spell check (requires `typos` installed)
typos

# Dependency audit (requires `cargo-deny` installed)
cargo deny check
```

**Always run `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
before considering work done.** Fix all warnings and test failures.

## Project structure

```
src/
├── main.rs            # Entry point, clap dispatch, anyhow context, error hints
├── cli.rs             # Clap derive: Cli, Commands enum, per-subcommand Args
├── error.rs           # GitreeError (thiserror), Result<T> alias, exit codes
├── types.rs           # Newtypes: BranchName, BareDir, SharedDir, WorktreePath
├── git.rs             # Typed Git wrapper: all `git` invocations go through here
├── repo.rs            # Wrapper discovery + methods (find wrapper root from CWD)
├── init.rs            # `gitree init` — bare clone setup
├── migrate.rs         # `gitree migrate` — pre-flight checks + atomic rename + linked-worktree relocation
├── worktree.rs        # `gitree add/remove/list/prune/where`
├── switch.rs          # `gitree switch/root` — prints cd command with shell escaping
├── foreach.rs         # `gitree foreach` — run command in all worktrees
├── pull.rs            # `gitree pull` — fetch + fast-forward main
├── status.rs          # `gitree status` — overview with dirty/ahead/behind
├── doctor.rs          # `gitree doctor` — health check
├── clean.rs           # `gitree clean` — prune + delete stale branches
├── env.rs             # `gitree env <shell>` — shell integration (gt, gtsw)
├── shared.rs          # .shared/ symlink fan-out + gitignore gotcha detection
├── completions.rs     # Shell completion (bash/zsh/fish + dynamic branch completion), manpage generation
└── format.rs          # ColorPolicy, WorktreeRow, text/JSON rendering

tests/
└── integration.rs     # End-to-end tests using assert_cmd + tempfile + real git

deny.toml              # cargo-deny config (licenses + advisories)
.typos.toml            # typos spell-checker config
```

## Architecture rules

### Git invocations

**All `git` commands go through `src/git.rs`.** Never call
`std::process::Command::new("git")` directly in other modules. Add a new
method to the `Git` struct instead. This centralizes error handling, verbose
output, and testability.

### Error handling

- **Library-level (`src/*.rs` except `main.rs`):** Return `error::Result<T>`
  (which is `Result<T, GitreeError>`). Use `?` for propagation. Add new
  variants to `GitreeError` in `error.rs` for new error categories.
- **Application-level (`main.rs` only):** Use `anyhow::Context` to add
  human-readable context to operations. The `run()` function returns
  `anyhow::Result<()>`. The `main()` function prints the full error chain
  (`{e:#}`) and extracts the root `GitreeError` for exit codes and hints.
- **Never use `.unwrap()` or `.expect()` outside tests.** Use `?` or
  `.unwrap_or_default()` for non-critical fallbacks.

### Newtypes

Domain primitives are wrapped in newtypes (`BranchName`, `BareDir`,
`SharedDir`, `WorktreePath`). These use `AsRef<Path>` / `AsRef<str>` for
borrowing — **no `Deref` impls** (Deref on newtypes is an anti-pattern per
the Rust API guidelines). When adding new domain types, follow the same
pattern.

### CLI structure

All subcommand definitions live in `src/cli.rs`. Use clap derive with:
- `#[command(alias = "...")]` for aliases
- `#[command(name = "__name", hide = true)]` for internal commands
- `clap::ValueEnum` for enum-valued arguments (e.g. `--color`)
- Doc comments become help text

### Shell integration

`src/env.rs` generates shell scripts for bash/zsh/fish/posix. The `gtr()`
function is a dispatcher: `gtr sw <branch>` does native `cd` (via `eval`),
everything else passes through to `gitree`. The `gtrsw` alias is `gtr sw`.
The function name is configurable via `gitree env <shell> --alias <name>`.

Each env script also wires up completions so `gtr`/`gtrsw` reuse the rules
installed by `gitree completion <shell>`:
- **fish:** `complete --command <alias> --wraps gitree`, and `gtrsw` is a
  function with `--wraps='gitree switch'` (not an alias) so branch-name
  completion fires — an alias would set `--wraps='gtr sw'`, and clap's fish
  completions only match the canonical subcommand name `switch`, not `sw`.
- **bash:** a lazy `_gtr_complete` loader sources the gitree bash completion
  file from standard paths on first use (the file may not be sourced yet when
  `gtr` is first completed), then delegates to `_gitree`.
- **zsh:** `compdef _gitree <alias>` (guarded on `compdef` availability).

### Output style

- **Git-style output:** clean, unindented lines. No boxes, panels, or
  excessive decoration.
- **Color:** respect `NO_COLOR`, `CLICOLOR_FORCE`. Use `ColorPolicy` enum
  (`Always`, `Never`, `Auto`). Auto-detect TTY via `std::io::IsTerminal`.
- **Errors to stderr,** data to stdout. Errors formatted as `error: <msg>`
  with optional `hint: <msg>` lines.
- **No emojis** unless explicitly requested by the user.

## Code style

### Rust conventions

- **Naming:** `snake_case` functions/variables, `UpperCamelCase` types.
- **Imports:** `use` statements at the top of the file, std first, then
  external crates, then `crate::` modules.
- **Documentation:** `///` doc comments on all public items. `//` only for
  non-obvious implementation notes. **Never add throwaway comments.**
- **Tests:** `#[cfg(test)] mod tests { use super::*; ... }` in the same file.
  Integration tests in `tests/`.
- **Error messages:** lowercase, no trailing punctuation (per `err-lowercase-msg`).
- **`#[must_use]`** on functions returning `Result` or newly-constructed
  values.
- **`#[non_exhaustive]`** on public error enums.

### Pattern preferences

- Iterators over explicit loops where natural.
- `?` over `match` on `Result`.
- Pattern matching over nested `if`.
- `impl` methods on types, not free functions.
- `From`/`TryFrom` for conversions (not `Into`).
- `serde::Serialize`/`Deserialize` for types that need JSON output.

### What to avoid

- `Deref` impls on newtypes (use `AsRef` instead).
- `.unwrap()` / `.expect()` in production code.
- `format!()` in hot paths (use `write!` where possible).
- Hand-rolled FFI (use crates instead).
- Procedural-style code (use iterators, pattern matching, traits).
- Comments unless non-obvious or explicitly requested.

## Git workflow

- **Never commit unless the user explicitly asks.**
- Before committing: inspect `git status`, `git diff`, `git log --oneline -10`.
- Stage only intended files. Never commit secrets.
- Write concise commit messages matching the repo style.
- Do not update git config, skip hooks, force-push, or create empty commits
  unless explicitly requested.

## Boundaries

- **Always do:** Run `cargo fmt && cargo clippy --all-targets -- -D warnings
  && cargo test` after making changes. Add tests for new functionality.
- **Always do:** Use the validated `BranchName` type when working with branch
  names in `worktree.rs` — never pass raw strings to git.
- **Always do:** Add new `git` operations as methods on `Git` in `git.rs`.
- **Ask first:** Before adding new dependencies to `Cargo.toml`.
- **Ask first:** Before changing the MSRV (`rust-version`).
- **Never do:** Call `std::process::Command::new("git")` outside of `git.rs`.
- **Never do:** Use `.unwrap()` or `.expect()` in non-test code.
- **Never do:** Add `Deref` impls to newtypes.
- **Never do:** Add throwaway comments. Only add comments when non-obvious
  or explicitly requested.
- **Never do:** Commit secrets, API keys, or `.env` files.

## Testing

- **Unit tests** live in-module (`#[cfg(test)] mod tests`).
- **Integration tests** live in `tests/integration.rs` and use `assert_cmd`
  to run the compiled binary against temporary git repos.
- Integration tests must set `GIT_COMMITTER_NAME`, `GIT_COMMITTER_EMAIL`,
  `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`, `GPG_TTY=""`, and
  `commit.gpgsign=false` to avoid signing agent issues.
- The `create_gitree_repo()` helper in `tests/integration.rs` builds a source
  repo and runs `gitree init` — reuse it for new tests.
- **Add tests for every new command or behavior.** Tests are not optional.

## Maintaining this file

**This AGENTS.md file must be kept up to date.** Update it whenever:

1. **The developer gives specific instructions that should always be
   followed.** If the user says "always do X" or "never do Y" or establishes
   a convention during the session, add it to the appropriate section
   (Architecture rules, Code style, Boundaries, etc.). Do not wait — update
   immediately.

2. **The project or processes evolve.** When you:
   - Add a new module to `src/` — add it to the Project structure section.
   - Add a new dependency — update the Tech stack section.
   - Change the build/test/lint commands — update the Build and test
     commands section.
   - Add a new CLI subcommand — update the Architecture rules and Project
     structure.
   - Change error handling patterns — update the Error handling section.
   - Establish a new convention or pattern — document it in Code style or
     Architecture rules.
   - Remove or rename something — update all references here.

3. **Review after each work session.** Before finishing a task, scan this
   file for anything that's now stale or missing. If the codebase has
   diverged from what's documented here, fix the documentation.

**When in doubt, update.** An over-documented AGENTS.md is better than an
under-documented one. The goal is that any coding agent picking up this
project cold can be productive immediately by reading this file.
