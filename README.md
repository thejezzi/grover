# grover

One ticket, many repositories, one command. A ticket is one unit of work, usually taken
from an issue tracker.

`grover` creates a folder per ticket and puts one git worktree of every affected repository
into it. A git worktree is a second checkout of a repository in its own directory; it shares
the repository's history and branches with the original checkout, so a ticket needs no stash
and no second clone. Files you always want in a ticket folder, such as an `AGENTS.md` for a
coding agent, are copied there as well.

```console
$ grover new PROJ-123 -r api,web
/home/you/work/tickets/PROJ-123
├── AGENTS.md
├── api/        # worktree on branch ticket/PROJ-123
└── web/        # worktree on branch ticket/PROJ-123
```

The only line on stdout — the program's data output — is the ticket folder, so the command
composes with the shell:

```console
$ cd "$(grover new PROJ-123 -r api,web)"
```

Nothing else reaches stdout. Progress, warnings and errors go to stderr, the message
channel.

## Why

A ticket rarely touches one repository. Without a tool, starting one means the same
handful of steps in every repository: `git worktree add`, invent a branch name, copy the
agent instructions, remember which path was for which repository. `grover` does that once,
from one config file, and can remove it again when the ticket is done.

It is deliberately small. `grover` does not commit, push, fetch or merge, and it never
deletes anything unless you call `grover rm`.

## Requirements

- `git` on `PATH`. `grover` drives the git CLI, so hooks and your git config behave exactly
  as they do in the terminal.
- Rust 1.85 or newer to build from source.

## Install

```console
$ cargo install --path grover-cli
```

or, using the Makefile:

```console
$ make install                    # installs to ~/.local/bin/grover
$ make install PREFIX=/usr/local  # system-wide, needs sudo
```

Confirm the installation:

```console
$ grover --version
grover 0.1.0
```

## First steps

`grover` reads a YAML config file. On a terminal, the quickest way to write one is the
wizard:

```console
$ grover config --init
? work_dir          ~/work/tickets
? branch_template   ticket/{ticket}
...
```

It asks for a ticket root (`work_dir`), a branch name pattern, the commit new branches
start from (the base ref), the folders that contain your repositories, and optional remote
repositories and files to copy. Every answer except `work_dir` has a default, and nothing
is written until all values pass validation. The wizard writes to the config that reading
would find; on Linux, a fresh machine uses `~/.config/grover/config.yaml`. `--config PATH`
writes it somewhere else, and `--force` overwrites an existing file. `grover config --path`
prints the path actually used.

The same settings can be written by hand; see
[docs/configuration.md](docs/configuration.md) for every field.

Then create a ticket:

```console
$ grover new PROJ-123 -r api
```

`PROJ-123` is the ticket name. `-r api` picks the repository `api`, which `grover` finds in
the folders named by `repo_dirs` or clones into `clone_dir`, the directory that holds the
clones, when the entry is a remote repository. The repository name is matched fuzzily, so
`-r api` also works when you type `API` or `ap`, provided the match is unambiguous.

Omit the ticket or `-r` and `grover` asks on a terminal; in a script it fails instead of
hanging, so `grover new` is safe inside agents and CI.

## The commands

| Command              | What it does                                                     |
| -------------------- | ---------------------------------------------------------------- |
| `grover new`         | Create a ticket folder with one worktree per selected repository |
| `grover list`        | Print the names of the existing ticket folders                   |
| `grover path`        | Print the folder of one ticket                                   |
| `grover rm`          | Remove a ticket: worktrees, branches, copies, then the folder    |
| `grover config`      | Edit the config, show it, or write a new one                     |
| `grover completions` | Print a shell completion script                                  |

`grover --help` lists them in the terminal, and `grover <command> --help` explains the
options of one command. The full reference is in [docs/commands.md](docs/commands.md).

## Shell integration

Bash, zsh, fish, elvish and PowerShell completions come from the binary:

```console
$ grover completions zsh > ~/.zfunc/_grover
```

A small function turns `new` into a `cd`, because `grover` prints the folder instead of
changing your shell's working directory:

```sh
gnew() { cd "$(grover new "$@")"; }
```

## Documentation

The [docs](docs/README.md) folder explains the whole tool:

- [Getting started](docs/getting-started.md) walks from installation to your first ticket.
- [Concepts](docs/concepts.md) explains tickets, worktrees, base refs and fuzzy matching.
- [Configuration](docs/configuration.md) documents every config field and the editors.
- [Commands](docs/commands.md) is the complete command-line reference.
- [Troubleshooting](docs/troubleshooting.md) covers the error messages and what to do.

## Status

Version 0.1.0. `new`, `list`, `path`, `rm`, `config` and `completions` are implemented.
A `status` command and per-worktree setup commands are planned, not shipped; the design
notes live in [PLAN.md](PLAN.md).

## Development

```console
$ make check    # cargo fmt --check, clippy with -D warnings, cargo test
$ make test
```

The workspace has two crates. `grover-lib` holds the workflow (config, repository
discovery, git calls, planning) and writes nothing to stdout; `grover-cli` holds arguments,
output and the interactive prompts. [AGENTS.md](AGENTS.md) has the house rules,
[PLAN.md](PLAN.md) the design and the roadmap.
