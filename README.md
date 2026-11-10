# gitree

A native git worktree workflow tool. gitree implements the bare-clone +
worktree layout, giving you a clean, symmetric structure where every branch is
a sibling directory.

gitree is **not** a replacement for git. You still use `git add`, `git commit`,
`git push`, etc. gitree only wraps `git worktree` operations with sensible
defaults, tab-completion, and the `.shared/` symlink automation.

## The layout

```
my-project/              ← wrapper (you never work here directly)
├── .bare/               ← git database (shared by all worktrees)
├── .git                 ← file: "gitdir: ./.bare"
├── .shared/             ← gitignored files symlinked into each worktree
│   └── .env
├── main/                ← worktree: main branch
│   ├── .env -> ../.shared/.env
│   └── src/
└── feature/my-feature/  ← worktree: feature/my-feature branch
    ├── .env -> ../.shared/.env
    └── src/
```

Each worktree is a fully independent working directory with its own
`node_modules`, build output, etc. The git history, stashes, and remotes are
shared across all worktrees via `.bare/`.

## Installation

```sh
cargo install --path . --locked
```

### Shell integration (recommended)

Add this to your shell config (`.bashrc`, `.zshrc`):

```sh
eval "$(gitree env bash)"
```

Fish:

```sh
gitree env fish | source
```

This defines:
- `gtr` — dispatcher: `gtr sw <branch>` changes directory natively, everything
  else passes through to `gitree` (e.g. `gtr add main`, `gtr ls`, `gtr pull`).
- `gtrsw` — alias for `gtr sw` (switch worktree with native `cd`).

The `gtr` and `gtrsw` names are configurable via `--alias <name>`:

```sh
eval "$(gitree env bash --alias gwt)"
```

### Shell completions

```sh
# Bash
gitree completion bash > ~/.local/share/bash-completion/completions/gitree

# Zsh
gitree completion zsh > "${fpath[1]}/_gitree"

# Fish
gitree completion fish > ~/.config/fish/completions/gitree.fish
```

The `gitree env` script (above) automatically wires `gtr` and `gtrsw` to these
completions, so `gtr l<TAB>` offers `list`, `gtrsw <branch><TAB>` offers branch
names, etc. The completion file must be installed separately (the `env` script
only adds the wiring; the candidate rules come from `gitree completion`).
Branch arguments of `add`, `remove`, `switch`, and `where` complete from
branch names for both canonical and aliased subcommand spellings.

### Manpages

```sh
gitree __mangen /usr/local/share/man/man1
mandb
```

## Quick start

### Option A: New repository

```sh
gitree init https://github.com/my-org/my-project.git
cd my-project
gitree add main
cd main
```

### Option B: Migrate an existing clone

```sh
cd my-project          # your existing regular clone
gitree migrate         # careful pre-flight checks, then conversion
cd main
```

The pre-flight rejects states it cannot preserve — initialized submodules,
split-index mode, `extensions.worktreeConfig`, in-progress operations
(merge/rebase/revert/cherry-pick/bisect), a nonstandard `core.worktree`, an
existing `.shared` entry, or content colliding with the new layout — and
`--force` does not bypass those hard rejections. Shared branch and stash
reflogs stay in the shared database; only the worktree's own `HEAD` reflog
moves.

## Commands

| Command | Alias | Description |
|---|---|---|
| `gitree init <url>` | | Create a new wrapper from a remote URL |
| `gitree migrate` | | Convert a regular clone to a gitree wrapper |
| `gitree add <branch>` | `a` | Add a worktree for a branch |
| `gitree remove <branch>...` | `rm` | Remove one or more worktrees |
| `gitree list` | `ls` | List all worktrees |
| `gitree prune` | | Prune stale worktree references |
| `gitree switch <branch>` | `sw` | Print a `cd` command for `eval` |
| `gitree where <branch>` | | Print the path of a worktree |
| `gitree root` | | Print the wrapper root directory |
| `gitree pull` | `pl` | Fetch and fast-forward main (all worktrees with `--all`) |
| `gitree foreach <cmd>` | `fe` | Run a command in every worktree |
| `gitree status` | `st` | Show status overview of all worktrees |
| `gitree doctor` | `doc` | Health check for the wrapper |
| `gitree clean` | | Remove stale worktrees and branches |
| `gitree env <shell> [--alias <name>]` | | Generate shell integration script |
| `gitree completion <shell>` | | Generate shell completions |

### `gitree init <url> [--name <dir>]`

Creates a wrapper directory, clones the repo as bare into `.bare`, writes the
`.git` pointer file, configures remote fetch refs, fetches, and creates
`.shared/`.

### `gitree migrate [--force] [--yes]`

Converts an existing regular clone into a gitree wrapper layout. Runs
extensive pre-flight checks (clean working tree, no untracked files, fsck
clean, local-only branch detection, plus hard rejects for states it cannot
preserve — initialized submodules, split-index, `extensions.worktreeConfig`,
in-progress operations, nonstandard `core.worktree`, an existing `.shared`
entry, layout collisions). Hard rejections apply even with `--force`.

### `gitree add <branch> [--new] [--base <ref>]`

Creates a worktree for `<branch>` in the wrapper directory. Symlinks all
`.shared/` entries into the new worktree (skipping paths tracked in the
worktree's index, so pending deletions are never replaced by symlinks).

- Without `--new`: the branch must exist (locally or on `origin`). If only on
  `origin`, a tracking local branch is created automatically. Upstream
  tracking (`branch.<name>.remote`/`.merge`) is configured when the branch
  has an `origin/<name>` counterpart, so plain `git pull`/`git push` work.
- With `--new`: creates a new branch. The base ref defaults to the current
  worktree's HEAD, or `main`/`master` if at the wrapper level.
- Re-running `add` for a branch whose worktree already exists is safe: it
  completes any missing `.shared` links (so an interrupted run can simply
  be retried). `--new` on an existing worktree still fails.

Tab-completes existing branch names (requires shell completion to be
installed).

### `gitree list [--json] [--color <always|never|auto>]`

Lists all worktrees with their branch, HEAD hash, path, and a `*` marker for
dirty worktrees.

```
abc1234  main           /home/user/project/main
def5678  feature/x      /home/user/project/feature/x *
```

### `gitree remove <branch>... [--delete-branch] [--force]`

Removes one or more worktrees. With `--delete-branch`, also deletes the
local branches.

Each argument may be a plain branch name, a directory-style name
(`branch/`, as offered by shell completion), or the worktree's path
(relative or absolute). Arguments are resolved against git's worktree
registry, so the *registered* worktree is removed even if ordinary git
operations left the branch checked out in a differently named directory.

### `gitree prune`

Cleans stale worktree references (e.g. after manually deleting a worktree
directory).

### `gitree switch <branch>`

Prints a `cd` command for the worktree. Like `remove`, the argument may also
be a directory-style name (`branch/`) or the worktree's path. Use with `eval`
or the `gtr` shell function:

```sh
# Manual
eval "$(gitree switch main)"

# With shell integration (recommended)
gtrsw main
```

### `gitree where <branch>`

Prints the filesystem path of a worktree. The argument may also be a
directory-style name (`branch/`) or the worktree's path.

### `gitree root`

Prints the wrapper root directory.

### `gitree pull [--all] [--branch <name>] [--autostash]`

Fetches from origin and fast-forwards the main worktree (or a specified
branch's worktree) if it is clean. If the worktree is dirty and there is
something to pull, the error names the branch and path. Use `--autostash`
to stash uncommitted changes before merging and pop them afterwards. If
the changes cannot be reapplied cleanly, gitree fails loudly and the stash
is kept — a conflicted autostash is never reported as success. Branches
that are ahead of origin (diverged) fail with a clear error; an
already-up-to-date branch is reported and left untouched.

With `--all`, every worktree that is behind its origin branch is
fast-forwarded. Dirty worktrees are skipped (with a note) unless
`--autostash` is given, which applies autostash per worktree. Worktrees
that are ahead of or diverged from origin are skipped and reported, as are
worktrees whose branch changed since the listing; branches without an
`origin/<branch>` upstream are ignored. `--all` cannot be combined with
`--branch`. Merge options inherited from `branch.<name>.mergeOptions` (e.g.
`--squash`) are neutralized per invocation so a fast-forward always moves
HEAD.

### `gitree foreach <command> [--parallel] [--only <glob>]`

Runs a shell command in every worktree:

```sh
gitree foreach 'npm install'
gitree foreach --parallel 'cargo test'
gitree foreach --only 'feature/*' 'make lint'
```

Sequential mode (default) runs worktrees in listing order and stops at the
first failure. `--parallel` uses a bounded worker pool (at most one worker
per core); output is buffered per worktree and printed grouped after all
commands finish, and failures are aggregated.

### `gitree status`

Shows an overview of all worktrees: branch, path, dirty count, ahead/behind
relative to origin. A worktree that cannot be inspected (missing directory,
corrupt index) is annotated with an error instead of silently reporting
zeros — in text as `(error: …)`, in JSON as a non-null `error` field.

### `gitree doctor`

Health check: git installed? `.bare/` exists? `.git` file correct? `git fsck`
clean? `.shared/` exists? Exits non-zero when any check fails, so scripts
can gate on it.

### `gitree clean [--force]`

Prunes stale worktree references, fetches with `--prune`, and identifies
local branches whose **configured upstream** is gone. Branches that were
never pushed (no upstream configured) are never considered stale. The
listing marks whether each branch is fully merged; `--force` prefers the
safe delete and only discards unmerged commits explicitly.

### `gitree env <shell> [--alias <name>]`

Generates shell integration script that defines `gtr` and `gtrsw` (or
`<alias>` and `<alias>sw` with `--alias`).

### `gitree completion <shell>`

Generates shell completion scripts (`bash`, `zsh`, `fish`, `elvish`,
`powershell`).

## The `.shared/` directory

Gitignored files (`.env`, `.editorconfig`, IDE configs) are not shared between
worktrees by default. gitree solves this with a `.shared/` directory in the
wrapper root:

1. Place shared files in `.shared/` (e.g. `.shared/.env`).
2. Every `gitree add` automatically symlinks all `.shared/` entries into the
   new worktree. Links are relative and point at the shared entry's name, so
   the wrapper can be moved and retargeted freely.
3. One source of truth — editing in one worktree changes it everywhere.

gitree refuses to link a shared path that is tracked in a worktree's index
(including pending deletions and sparse-checkout skips), and warns when a
linked entry is **not effectively ignored** in that worktree — an ignore
rule must exist in the worktree's own `.gitignore` (or `.git/info/exclude`
/ `core.excludesFile`); the wrapper-level `.gitignore` is invisible from
inside worktrees. Verify with `git check-ignore <name>` from a worktree.

Two entries are treated specially:

- `.shared/.gitignore` is never linked: Git deliberately does not follow
  symlinked `.gitignore` files, so sharing it would silently disable those
  rules.
- Shared files are still ordinary local files. Sharing mutable dependency
  directories (`node_modules`, build output) makes every worktree write the
  same state — usually not what you want.

## Daily workflow

```sh
# Start a new feature
gitree add feature/my-feature --new
gtrsw feature/my-feature
npm install

# Switch context (no stashing, no WIP commits)
gtrsw main

# Code review
gitree add pr-review      # checks out origin/pr-review
gtrsw pr-review
npm install
# ... review, test ...
gitree remove pr-review

# Clean up
gitree remove feature/my-feature --delete-branch
gitree prune
```

## Development

### Build

```sh
cargo build --release
```

### Test

```sh
cargo test
```

Integration tests shell out to real `git` and create temporary repositories.

### Lint

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

### Spell check

```sh
typos
```

### Dependency audit

```sh
cargo deny check
```

## License

MIT
