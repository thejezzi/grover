# Configuration

`grover` has no database and no hidden state. Everything it needs comes from one YAML file,
and everything it does is derived from that file plus what exists on disk. This page
describes where the file lives, how the paths inside it are resolved, every field, and the
two interactive ways of writing it.

If you have not created a config yet, the tutorial in [Getting started](getting-started.md)
walks through the wizard. The example in this repository,
[`grover.yaml.example`](../grover.yaml.example), is the same format and is kept valid by a
test.

## Where the file lives

Reading uses this order, and the first file that exists wins:

1. the path given with the global `--config PATH` option,
2. `grover.yaml` in the current directory,
3. `$XDG_CONFIG_HOME/grover/config.yaml`, where `XDG_CONFIG_HOME` falls back to `~/.config`.
   On Linux that is the usual location. On macOS, `dirs` resolves the config directory to
   `~/Library/Application Support`, so the path is
   `~/Library/Application Support/grover/config.yaml`; `grover config --path` prints the
   path actually used.

`--config` accepts a relative path, which is resolved against the current directory, and must
name an existing file when a command reads the config. When the file is missing, the error
says which path was expected.

Writing follows almost the same rule, with one difference: `grover config --init` writes to
`--config` when given, else to an existing config found by the reading rule, else to the
`dirs` config path (on Linux, `$XDG_CONFIG_HOME/grover/config.yaml`). An existing file is
never overwritten without `--force`. `grover config --path` prints
the location that reading would use, which is a quick way to check what is actually in
effect.

Because `./grover.yaml` comes before the user-wide file, a repository can carry a project
config that wins while you are inside it. All commands accept `--config` in front of the
subcommand (`grover --config ci.yaml new ...`) or after it (`grover new --config ci.yaml`).

## How paths are resolved

Every path that is resolved against the config file — `work_dir`, `repo_dirs`, `clone_dir`
and the `from` of a `copy` entry — may be relative, may start with `~`, or may be absolute.
Relative paths are resolved against the directory of the config file, not against the
directory you run `grover` from. That makes a checked-in config portable: the same file
works from any subdirectory of the project. `~` always means the home directory of the
current user.

Internally every path is absolute, so the rest of the tool never has to think about the
current directory. `grover config --show` prints the resolved config with absolute paths,
which is the quickest way to see where a `~` or a relative entry actually points.

One path is deliberately not resolved this way: the `to` of a `copy` entry is relative to
the ticket folder, not to the config file. It must be a relative path; an absolute `to` is
rejected at validation, because `rm` would otherwise delete a file outside the ticket folder.

## Validation

The file is validated when it is read and again before it is written. A config is rejected
when any of these is true:

- `work_dir` is missing or empty,
- `branch_template` does not contain `{ticket}`,
- `base` contains no ref at all,
- neither `repo_dirs` nor `repos` is set,
- `repos` is set but no fallback clone directory could be determined,
- a `repos` entry has no name and none can be derived from its URL,
- a `copy` entry has an empty `from`,
- a `copy` entry has an absolute `to`.

Unknown fields are an error too, at every level of the file. A misspelled `repo_dir` does not
silently turn into an empty list; the parse fails and names the file. The same applies to a
misspelled key inside a `repos` or `copy` entry.

## Fields

| Field | Kind | Default | Required |
| --- | --- | --- | --- |
| `work_dir` | path | — | yes |
| `branch_template` | string | `ticket/{ticket}` | no |
| `base` | string of refs | `HEAD` | no |
| `repo_dirs` | list of paths | empty | no |
| `clone_dir` | path | `~/.grover/repos` | no |
| `repos` | list of entries | empty | no |
| `copy` | list of entries | empty | no |
| `setup` | list of shell commands | empty | no |

`work_dir` is the only field without a default, and it plus at least one of `repo_dirs` or
`repos` must be present, or the config does not pass validation.

### work_dir

The directory in which ticket folders are created. It does not have to exist; `grover new`
creates it, including missing parents. `grover list` on a `work_dir` that does not exist
returns an empty list rather than an error, because "no tickets yet" is a normal state.

Every direct subdirectory of `work_dir` is treated as a ticket by `list`, `path` and `rm`.
That is why the field is required: without it, the ticket folders would end up somewhere
unintended, most likely next to the config file.

### branch_template

The name pattern for the branch of a new worktree, a second checkout of a repository (see
[Worktrees and branches](concepts.md#worktrees-and-branches)). The token `{ticket}` is
replaced by the ticket slug, the normalized ticket name (see
[Tickets and slugs](concepts.md#tickets-and-slugs)), so `ticket/{ticket}` produces
`ticket/PROJ-123`. The token is mandatory; a template without it is rejected at validation
time because every ticket would otherwise share one branch name.

The template can contain slashes, which git reads as branch name separators. It is a plain
string replacement, not a template language: there is exactly one token and no conditionals.

### base

The commit a new branch starts from, written as one or more refs joined by `|`. A ref is a
git name for a commit, such as a branch name or `HEAD`. For each repository, the refs are
tried from left to right and the first that exists there is used.
`main|master|HEAD` is a common value: repositories on `main` follow `main`, older ones fall
back to `master`, and anything else uses whatever is checked out.

`HEAD` is the default and always resolves in a repository with at least one commit. If none
of the refs exists in a repository, the run fails and names the refs it tried. A base is
only consulted when the branch is new; an existing branch is reused at its current commit.

### repo_dirs

The folders whose direct children are searched for git repositories. A child with a `.git`
entry is picked up; its folder name becomes the repository name. Nothing is cloned or
fetched from these directories, and nested repositories are not searched recursively.

A missing or unreadable `repo_dirs` entry is a warning, not an error: the rest of the
directories are still used, so a config can list a directory that only exists on one
machine. If two entries contain a repository with the same name, the earlier entry wins.

### clone_dir

The destination for clones of repositories from `repos`. If a selected remote repository is
not present there yet, it is cloned into `clone_dir/<name>`. The default is
`~/.grover/repos`, so the folders survive between runs and are reused by later tickets.

The directory is created on demand. If the config has `repos` entries and neither `clone_dir`
nor a home directory is available, validation fails, because there would be nowhere to clone
to.

### repos

Repositories that are cloned only when they are selected for a ticket. Each entry has a
`url`, which is passed to `git clone`, and an optional `name`:

```yaml
repos:
  - url: "git@github.com:org/api.git"
    name: api                    # optional
```

Without `name`, the name is the last segment of the URL without a trailing `.git`, so the
example above is `api` either way. An entry whose URL yields no usable name is a config
error. Local repositories always win over a remote entry with the same name.

Cloning happens inside the run that first needs the repository. Because git is called with
`GIT_TERMINAL_PROMPT=0`, a repository that asks for credentials fails with a git error
instead of hanging on a password prompt.

### copy

Files copied into every ticket folder. This is where the `AGENTS.md` in the opening example
comes from:

```yaml
copy:
  - from: "~/templates/AGENTS.md"
    to: AGENTS.md                # optional
```

`from` is the source file, resolved like every other config path. `to` is the destination
relative to the ticket folder; without it, the file keeps its own name. An absolute `to` is
rejected, because it would leave the ticket folder. Parent directories of `to` are created
when needed, so `to: docs/AGENTS.md` works.

Copying targets the ticket folder, not the individual worktrees. A copy target that already
exists is skipped, never overwritten. Directories are not copied, and a `from` that does not
exist fails when the copy is attempted, after the worktrees have been created.

### setup

`setup` is an optional list of shell commands run with `sh -c` after each new worktree is
created. Commands run in order, with the worktree as the current directory. Their stdout is
sent to grover's stderr, while their stderr is inherited. `GROVER_REPO` is the repo name and
`GROVER_TICKET` is the normalized ticket
slug. Existing worktrees and files copied into the ticket folder do not trigger setup.

A failed command prints a warning with its exit status; later commands still run, and the
worktree is kept. `--dry-run` logs the commands without running them, so do not put secrets
in command strings. Commands run with your permissions. Only use setup entries from
a config you trust.

```yaml
setup:
  - "npm install"
```

## A complete example

```yaml
# Required: where ticket folders are created.
work_dir: "~/work/tickets"

# Branch name; `{ticket}` is replaced by the ticket slug.
branch_template: "ticket/{ticket}"

# First ref that exists in a repository is the base of a new branch.
base: "main|master|HEAD"

# Local repositories: direct children of these folders.
repo_dirs:
  - "~/code"
  - "~/work/vendored"

# Where clones from `repos` land; this is the default.
clone_dir: "~/.grover/repos"

# Remote repositories, cloned on first use.
repos:
  - url: "git@github.com:org/api.git"
    name: api
  - url: "https://github.com/org/web.git"

# Files copied into the ticket folder.
copy:
  - from: "~/templates/AGENTS.md"
    to: AGENTS.md

# Commands run after each new worktree is created.
setup:
  - "npm install"
```

A minimal config is much shorter; the only non-negotiable part is `work_dir` plus at least
one repository source:

```yaml
work_dir: "~/work/tickets"
repo_dirs:
  - "~/code"
```

## Working with the config

`grover config --show` prints the resolved config as YAML and exits; it is the command to
use in a script, a diff or a support question. `grover config --path` prints only the file
that would be read.

Without a flag, `grover config` opens an interactive editor. It needs a terminal, because it
is a menu, and it looks like this:

```text
? Config · /home/you/.config/grover/config.yaml
❯ work_dir          /home/you/work/tickets  Required; ticket folder parent.
  branch_template   ticket/{ticket}          Branch pattern, e.g. ticket/{ticket}.
  base              main|master|HEAD         Fallback refs in order.
  repo_dirs         2 entries                Search direct child folders for local repos.
  clone_dir         /home/you/.grover/repos  Remote clone folder; empty clears it.
  repos             0 entries                Remote repos are cloned when selected.
  copy              1 entry                  Copy files into each ticket folder.
  setup             0 entries                Run commands after each new worktree.
  ← exit
```

The third column is the one-sentence help that the field also shows under its prompt.
Choosing a scalar field asks for a new value: the current one is shown in grey as the
default, Enter keeps it, typing replaces it. Esc in a field returns to the menu without
changing anything, and Esc in the menu leaves the editor. Choosing a list opens a small
submenu with the entries, an entry for adding one, and an entry for going back; picking an
existing entry removes it. There is no in-place edit of a list entry, only add and remove.

The editor writes after every menu action, so leaving the menu is not a commit step. Before
each write the new state is validated by writing it and reading it back; if anything is
wrong, the previous file is restored and the problem is reported as a warning. If the config
is a symlink, the link is kept and the file it points to is written.

Two consequences are worth knowing. The editor writes the resolved paths as absolute, so a
config with `~` or relative entries becomes machine-specific after the first save; `copy.to`
stays as it is, because it is relative to the ticket folder. And the editor writes fresh
YAML, so comments in the file are lost. If you keep the config in a dotfiles repository or
share it with a team, edit it in your editor and use the interactive one only for quick
changes.

`grover config --init` is the other direction: it starts from nothing, asks each numbered
field in turn, including the `setup` command list, and writes the result only after a final
`write <path>?` question. It never modifies an existing config unless you pass `--force`;
with `--force` a loadable existing config pre-fills the defaults. Esc goes back one field and
keeps what you typed. Like the editor, it needs a terminal; without one it fails and names
the file it would have written. A declined final question or an invalid value leaves no new
file behind and keeps an existing one untouched.

For one change there is `grover config --set <field>[=<value>]`, which edits a single scalar
field and writes the file without opening the editor. Without `=<value>` it prompts on a
terminal; an empty value clears `clone_dir`. Only the fields shown as scalars in the menu are
supported, lists stay in the editor, and the config file must already exist.

Values you enter with the default kept are written as well, so the result is a complete
config. Lists that end up empty are left out entirely, which keeps the file short; a valid
fresh file still contains at least one entry in `repo_dirs` or `repos`, because a repository
source is required.
