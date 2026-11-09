# Getting started

This page walks you from a machine with `git` installed to a ticket folder you can work in.
It introduces each piece of vocabulary the first time it is needed. The reference pages
[Configuration](configuration.md) and [Commands](commands.md) go deeper, but you do not need
them to follow along.

## Before you start

You need `git` on your `PATH` and a terminal. `grover` calls the `git` binary instead of
linking a git library, so every repository behaves the way it does in your terminal,
including your global git config and your hooks. You also need a directory in which your
repositories already exist, or URLs of repositories to clone.

`grover` itself is a Rust binary. Install it from the checkout with

```console
$ cargo install --path grover-cli
```

or build from the checkout with `make install`, which puts the binary in `~/.local/bin` by
default. The command is then simply `grover`:

```console
$ grover --version
grover 0.1.0
```

## Write a config

`grover` keeps its settings in a YAML file. It looks for the file in two places, in this
order: `grover.yaml` in the current directory, then
`$XDG_CONFIG_HOME/grover/config.yaml`. `XDG_CONFIG_HOME` is the standard variable for your
configuration directory; on Linux, when it is unset, which is the normal case, the path is
`~/.config/grover/config.yaml`. On macOS, `dirs` uses
`~/Library/Application Support/grover/config.yaml`; `grover config --path` prints the actual
path.

The easiest way to create the file is the wizard:

```console
$ grover config --init
? work_dir  /home/you/work/tickets
? branch_template  ticket/{ticket}
? base  HEAD
...
```

The wizard asks for one value at a time, in this order: `work_dir`,
`branch_template`, `base`, `repo_dirs`, `clone_dir`, `repos`, `copy` and
`setup`. `work_dir` is the only value without a default: it is the folder
where ticket folders will be created, for example `/home/you/work/tickets`.
The other questions show their defaults in grey and accept them with Enter;
typing replaces them. Esc goes back one field and keeps your answer.
The four lists (`repo_dirs`, `repos`, `copy`, `setup`) are menus: choose
`+ add` to append a value, then leave through the back entry. `setup` commands
run in order after each new worktree. Their output goes to stderr, so stdout
stays the ticket path. The last question is `write <path>?`; nothing is written
before it, so declining leaves no half-finished file behind.

If you prefer to write the file yourself, put this in `~/.config/grover/config.yaml` (or in
`./grover.yaml` next to your work):

```yaml
work_dir: "~/work/tickets"

repo_dirs:
  - "~/code"

copy:
  - from: "~/templates/AGENTS.md"
    to: AGENTS.md
```

`repo_dirs` lists the folders whose child folders are searched for git repositories. In this
example, a repository at `~/code/api` becomes available under the name `api`. The `copy`
entry places a file in each new ticket folder; it assumes that `~/templates/AGENTS.md`
exists, so drop that block if you do not have such a file. Both fields are explained in
[Configuration](configuration.md).

A config is only accepted when it makes sense as a whole: there must be at least one source
of repositories, `work_dir` must be set, and the branch template must contain `{ticket}`.
Mistakes in the file, including a misspelled field name, are reported as errors rather than
silently ignored.

## Create your first ticket

A ticket is a work item with a name and a folder. Run

```console
$ grover new PROJ-123 -r api
```

Here `PROJ-123` is the ticket name, and `-r api` selects the repository `api` from the
repositories found through `repo_dirs`. Repository names are matched fuzzily, so `-r API` or
a small typo works as long as exactly one repository fits. With several repositories you can
repeat `-r`, pass a comma-separated list such as `-r api,web`, or take everything at once
with `--all`.

`grover` then does five things:

1. It normalizes the ticket name into a slug, which is the name used for the ticket folder
   and the branch. `PROJ 123/foo` becomes `PROJ-123-foo`; `PROJ-123` stays `PROJ-123`.
2. It builds the list of known repositories, then resolves your selection against that list.
   An unknown or ambiguous name stops the run here, before anything is written.
3. It creates the ticket folder, `work_dir/PROJ-123` in this example.
4. For each selected repository it adds a worktree — a second checkout of that repository,
   with its own files — at `<ticket folder>/<repo name>`, on a new branch named after
   `branch_template`. The default template is `ticket/{ticket}`, so the branch is
   `ticket/PROJ-123`.
5. It runs the configured `setup` commands in each new worktree.
6. It copies the files from the `copy` list into the ticket folder.

The result looks like this:

```text
~/work/tickets/PROJ-123
├── AGENTS.md          # copied from ~/templates/AGENTS.md
└── api/               # worktree, on branch ticket/PROJ-123
```

The ticket folder is the only thing on stdout, the data channel. Everything about the run —
the worktrees it created, the files it copied, warnings, errors — goes to stderr, the message
channel. That separation is why you can put the command into a subshell, the `$(...)` part of
the next command:

```console
$ cd "$(grover new PROJ-123 -r api)"
```

If you omit the ticket name, or omit `-r` and `--all`, `grover` asks for the missing part
on a terminal. It only asks when stdin is a terminal; in a script or inside an agent there
is no prompt, and the command fails with a message instead of waiting forever.

## The daily loop

Once tickets exist, four commands cover the daily loop. `grover list` prints the names of
your ticket folders, one per line and sorted, which makes it useful for a quick look and for
scripting:

```console
$ grover list
PROJ-123
PROJ-124
```

`grover path` prints the folder of a single ticket, which saves remembering where each one
lives:

```console
$ grover path PROJ-123
/home/you/work/tickets/PROJ-123
```

With an argument the ticket may be abbreviated or misspelled, as long as one existing ticket
matches. Without an argument, `grover path` looks at the current directory: if you stand
somewhere inside a ticket folder, it prints that ticket. So `cd "$(grover path)"` also works
from inside a ticket.

`grover status` shows what is happening in the worktrees of a ticket: the branch, the
distance to the upstream, and the number of changed files. It reads the worktrees directly,
so it works from anywhere inside the ticket:

```console
$ grover status PROJ-123
repo   branch             ahead/behind  dirty
api    ticket/PROJ-123    +2/-0         3 files
web    ticket/PROJ-123    -             clean
```

The `-` under `ahead/behind` means the branch has no upstream yet, which is normal for a
fresh ticket branch.

`grover rm` removes a ticket again. That means, in this order: each worktree and its branch,
the files `grover` copied in, and finally the ticket folder itself:

```console
$ grover rm PROJ-123
```

Deletion is the one place where `grover` touches data you might care about, so it is
deliberately cautious. It refuses to guess a ticket name; it never removes a dirty worktree
or an unmerged branch without `--force`; and it only removes the ticket folder when nothing
else is left in it. Anything unknown — a notes file you added, a nested repository — keeps
the folder alive. If you want to see what would happen without doing it, add `--dry-run`:

```console
$ grover rm PROJ-123 --dry-run
```

The [Commands](commands.md) page explains the removal rules in full, because they are the
part of the tool where knowing the details pays off.

## Optional: shell completion

`grover` can print a completion script for bash, zsh, fish, elvish and PowerShell. Load
it the way your shell expects, for example in zsh with the `_grover` file in a directory on
`$fpath`, the list of directories zsh searches for completions:

```console
$ grover completions zsh > ~/.config/zsh/functions/_grover
```

There is no `cd` built into `grover`, and there cannot be: a child process cannot change
the working directory of its parent shell. The subshell form above is the intended way to
combine creation and navigation.

## Where to go next

- [Concepts](concepts.md) explains what worktrees and base refs are, and why `grover` asks
  in some situations but fails in others.
- [Configuration](configuration.md) shows the full config, including remote repositories
  that are cloned on demand.
- [Troubleshooting](troubleshooting.md) helps when a command stops with a message about a
  config, a repository name or a git branch.
