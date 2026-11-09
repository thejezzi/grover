# Command reference

This page documents every command, every option, the environment variables and the exit
codes. It assumes the vocabulary from [Concepts](concepts.md): ticket, slug, repository,
worktree, branch, base ref. If a sentence here uses one of those terms before you have read
that page, the link is the place to go.

`grover` is a single binary with seven subcommands:

```text
grover new [<ticket>] [-r|--repo <name>]... [-a|--all]
           [-t|--title <text>] [--slug-from-title] [-n|--dry-run]
grover list
grover path [<ticket>]
grover status [<ticket>]
grover rm [<ticket>] [-f|--force] [-n|--dry-run]
grover config [-p|--path] [-s|--show] [-i|--init [--force]]
              [--set <field>[=<value>]]
grover completions <shell>
```

Every command also accepts the global options below. `grover --help` prints the summary,
`grover <command> --help` prints the options of one command, and `grover help <command>` is
the same as the latter.

## Global options

These options belong to the program rather than to one subcommand. They may appear before or
after the subcommand name.

| Option | Meaning |
| --- | --- |
| `-c`, `--config <PATH>` | Use this config file instead of the default search |
| `-v`, `--verbose` | More output; `-v` is debug, `-vv` is trace |
| `-q`, `--quiet` | Warnings and errors only |
| `-h`, `--help` | Print help and exit with 0 |
| `-V`, `--version` | Print the version and exit with 0 |

`--config` is described in [Configuration](configuration.md#where-the-file-lives). A missing
or invalid file is an error for the commands that read it. `grover completions` does not
read the config at all, and `grover config --init` creates a config instead of reading one.

Verbosity changes what is written to stderr, never what is written to stdout. The default
level reports each created folder, worktree, setup command and copy as one line, plus a
summary line with the counts of created, skipped and cloned items. `-q` keeps warnings and
errors and drops those progress lines; `-v` adds debug output. Setting the environment
variable `GROVER_LOG` overrides both flags; its value is a log filter such as `debug` or
`grover_lib=trace`.

`git` is called with `GIT_TERMINAL_PROMPT=0`, so a repository that needs interactive
credentials fails instead of waiting for input. Setup commands inherit `GROVER_REPO` and
`GROVER_TICKET`; see [Environment variables](#environment-variables).

## Prompts

Several commands ask for a missing value instead of failing. That only happens when stdin is
a terminal. When stdin is a pipe or `/dev/null` — a script, a CI job, a coding agent — the
commands never ask; they fail with a message that says what to pass. This is the one rule
behind every interactive behavior in `grover`, and it is why the same command line is safe
in both worlds.

Prompts are rendered on stderr, so stdout keeps carrying only the result. In a text prompt,
Enter confirms and Esc goes back; the default value, when there is one, is shown in grey and
is used when you press Enter without typing. Path prompts (`work_dir`, `repo_dirs`,
`clone_dir`, `copy.from`) complete with Tab: the entries of the typed folder are listed as
you type, `↑`/`↓` pick one, and Tab completes to the longest common prefix when none is
picked. A typed `~` stays a `~` in the suggestions.

Selection prompts have two modes. Normally `j`/`k` (or `↑`/`↓`) move, `g`/`G` jump to the
first and last entry, and letters do nothing else. Press `/` to search: what you type filters
the list as a case-insensitive substring, Enter keeps the filter and returns to moving around,
Esc clears it. In a multi-selection, Space toggles the entry under the cursor, `l` selects all
entries and `h` selects none; Enter confirms the selection.

Esc means "one step back": in the config editor it returns from a field to the menu and from
the menu to the shell, in the `--init` wizard it returns to the previous field and keeps the
answer you typed. Ctrl-C is different: it aborts the command with exit code 1. For `new` and
`rm` nothing has been written at that point, and the error text says `aborted, nothing
written`. The config editor saves after every action, so Ctrl-C there keeps the actions you
already confirmed.

## grover new

```text
grover new [<ticket>] [-r|--repo <name>]... [-a|--all]
           [-t|--title <text>] [--slug-from-title] [-n|--dry-run]
```

`new` creates the ticket folder, a worktree for each selected repository — a second checkout
of that repository — and the copied files. It prints the ticket folder as the only line on
stdout:

```console
$ grover new PROJ-123 -r api
/home/you/work/tickets/PROJ-123
```

The folder is printed after the work is done, and it is printed in a dry run too. Because
the rest of the output goes to stderr, the usual way to create a ticket and enter it is a
subshell:

```console
$ cd "$(grover new PROJ-123 -r api)"
```

### Arguments and options

| Argument or option | Meaning |
| --- | --- |
| `<ticket>` | Ticket name; asked on a terminal when omitted |
| `-r`, `--repo <name>` | Repository name, fuzzy; repeat or separate with commas |
| `-a`, `--all` | Every known repository; cannot be combined with `-r` |
| `-t`, `--title <text>` | Ticket title, e.g. `"Login redirect loop"` |
| `--slug-from-title` | Put a slug of the title into the branch name; needs `--title` |
| `-n`, `--dry-run` | Print the plan (folders, worktrees, copies) and write nothing |

The repository names are matched with the fuzzy ladder described in
[Concepts](concepts.md#how-names-are-matched), so `-r API`, `-r api` and a small typo all
select `api` when it is the only match. Duplicate selections are harmless; a repository that
is named twice is created once. With `--all`, every repository found through `repo_dirs` and
`repos` is used, and remote repositories are cloned as part of the run.

Giving `--all` together with `-r` is a command-line error: clap rejects the combination with
exit code 2 before `grover` reads the config.

### What it does

The command first resolves and then writes. Resolving covers the slug (the normalized ticket
name), the repository discovery, the fuzzy selection and the target paths; it fails without
touching the disk if the ticket name has no usable characters, a repository name is unknown
or ambiguous, or the config is invalid. Writing creates the ticket folder (including missing
parent folders), clones remote repositories that are not present yet, adds one worktree per
repository, runs `setup` for each new worktree, and then copies the files from `copy`.

With `create_branch: true`, each worktree gets the branch from `branch_template`, starting at
the first existing ref of `base` in that repository — a ref is a git name for a commit — or is
reused if a branch with that name already exists. The default is `create_branch: false`, which
adds the worktree detached at that ref instead: no branch per repository. The whole order is:
ticket folder, then for each repository the clone, worktree and setup commands, then the
copies.

Paths that already exist are skipped with a warning, and the run still ends with exit code
0. This makes the command idempotent, and it allows a later call to add a repository to an
existing ticket:

```console
$ grover new PROJ-123 -r web    # adds web/ to the ticket that already has api/
```

Nothing is ever overwritten. Existing files at a copy destination stay untouched, and an
existing worktree is left as it is.

### A branch name that says what the ticket is about

`branch_template` turns the ticket into a name, so `PROJ-123` becomes `ticket/PROJ-123`. Pass
a title and ask for a slug of it, and the branch says more:

```console
$ grover new PROJ-123 -r api --title "Login redirect loop" --slug-from-title
$ git -C ~/work/tickets/PROJ-123/api branch --show-current
ticket/PROJ-123-login-redirect-loop
```

The slug is built locally, without a model: lowercase, non-alphanumeric characters become
`-`, umlauts become `ae`/`oe`/`ue`/`ss`, filler words such as `a`, `the`, `of`, `der`, `und`
are dropped, and the result keeps at most five words and 48 characters. The template's shape
is unchanged, so `feat/{ticket}` yields `feat/PROJ-123-login-redirect-loop`. The ticket
folder stays `PROJ-123`: the title affects the branch, not the path.

Two things to know: `--slug-from-title` without `--title` is a command-line error, and with
`create_branch: false` — the default — the title is ignored with a warning, because no branch
is created to carry the name. A title that leaves no usable characters after cleaning, such
as `---`, produces the same warning and the plain ticket branch.

### Dry run

`--dry-run` prints every step it would take and writes nothing: no folder, clone, worktree,
setup command or copy. Setup commands are logged but not run. The progress lines are followed
by `dry-run: nothing written`, and the ticket folder is printed on stdout. The summary line
counts what would have been created. A dry run is the safe way to check a new config or an
unfamiliar repository selection.

### Examples

```console
$ grover new PROJ-123 -r api                 # one repository
$ grover new PROJ-123 -r api,web             # two, comma separated
$ grover new PROJ-123 -r api -r web          # the same, repeated
$ grover new PROJ-123 --all                  # every known repository
$ grover new "PROJ 123/foo" -r api --dry-run # slug: PROJ-123-foo
$ grover new PROJ-123 -r api \
    --title "Login redirect loop" --slug-from-title  # ticket/PROJ-123-login-redirect-loop
```

The ticket name is the one from your issue tracker; it does not have to be filesystem-safe.
Spaces and slashes become `-` in the slug, which is then used for both the folder and the
branch. See [Tickets and slugs](concepts.md#tickets-and-slugs) for the exact rule.

## grover list

```text
grover list
```

`list` prints the names of the ticket folders directly below `work_dir`, one per line and
sorted, and nothing else:

```console
$ grover list
PROJ-123
PROJ-124
```

Only directories count as tickets. A stray file such as `notes.txt` in `work_dir` is
ignored, and subdirectories of ticket folders are never listed. A `work_dir` that does not
exist yet produces an empty output and exit code 0, because an empty list is a valid answer
before the first ticket exists.

The command does not scan repositories, read worktrees or call git. It only reads one
directory, which keeps it fast and makes it useful in a prompt or a script. The config must
still exist and be valid.

## grover path

```text
grover path [<ticket>]
```

`path` prints the folder of one ticket, one line on stdout:

```console
$ grover path PROJ-123
/home/you/work/tickets/PROJ-123
```

With an argument, the name is normalized to a slug like everywhere else, then matched
against the existing tickets. An exact match wins; otherwise the fuzzy ladder is used, so an
abbreviated or slightly misspelled name works when exactly one ticket fits. No match is an
error that suggests similar names; several matches are an error that lists the candidates.
Nothing is created, and an ambiguous name is never decided by chance.

Without an argument, `path` looks at the current working directory instead: it must be
inside `work_dir` or below it, and the first path component below `work_dir` is the ticket.
Standing in `/home/you/work/tickets/PROJ-123/api/src` therefore prints
`/home/you/work/tickets/PROJ-123`. Standing directly in `work_dir` or outside it is an
error.

The heart of the command is the subshell, which is how `grover` compensates for being unable
to change its parent shell's directory:

```console
$ cd "$(grover path PROJ-123)"
$ cd "$(grover path)"           # from anywhere inside a ticket
```

The no-argument case compares the current directory with `work_dir` as written, without
resolving symbolic links. If `work_dir` is reached through a symlink, the no-argument form
can report that the directory is not below `work_dir`; pass the ticket name instead.

## grover status

```text
grover status [<ticket>]
```

`status` prints the state of every worktree in a ticket as a table:

```console
$ grover status PROJ-123
repo   branch             ahead/behind  dirty
api    ticket/PROJ-123    +2/-0         3 files
web    ticket/PROJ-123    -             clean
```

| Column | Meaning |
| `repo` | The folder name of the worktree, which is the repository name |
| `branch` | The branch checked out there; `HEAD` when HEAD is detached |
| `ahead/behind` | Commits ahead of and behind the upstream, such as `+2/-0`; `-` without one |
| `dirty` | `clean`, or the number of changed files reported by `git status` |

The argument is matched like `path`: normalized to a slug, then exact, then fuzzy when
exactly one ticket fits. Without an argument the ticket is the one above the current
working directory, so `grover status` works from anywhere inside a ticket. Only the direct
children of the ticket folder that are worktrees are read; a notes file or a foreign
directory is ignored.

Each row is three git calls: `git rev-parse --abbrev-ref HEAD` for the branch,
`git status --porcelain` for the changed files, and
`git rev-list --left-right --count @{u}...@` for the upstream distance. A fresh ticket
branch has no upstream, so the `ahead/behind` column shows `-`; that is the normal case,
not an error. The table goes to stdout because it is the result of the command. There is no
`--json` and no column option; the output is meant to be read, not parsed.

## grover rm

```text
grover rm [<ticket>] [-f|--force] [-n|--dry-run]
```

`rm` removes a ticket: its worktrees, their branches, the files `grover` copied in, and
finally the ticket folder. It prints nothing on stdout; the steps it takes and everything it
skips go to stderr. This is the only command that deletes anything, so it is deliberate
about names and cautious about data.

| Argument or option | Meaning |
| --- | --- |
| `<ticket>` | Ticket name; picked from a list on a terminal when omitted |
| `-f`, `--force` | Remove dirty worktrees and unmerged branches too |
| `-n`, `--dry-run` | Print the plan and remove nothing |

The ticket name is normalized to a slug and then must match an existing ticket exactly.
Fuzzy matching is never used to choose something to delete, because a candidate list cannot
undo a wrong choice; an unknown name is an error with the usual "similar" suggestions and
changes nothing. Without an argument on a terminal, `rm` shows a filterable list of existing
tickets. Without an argument and without a terminal, it fails with the message
`missing ticket: pass <ticket>`. The current directory is never used to infer a ticket.

### What is removed, and in which order

First the worktrees. `grover` takes each child of the ticket folder whose `.git` entry is a
file, which is how a worktree marks itself; ordinary files and other directories are
ignored. For each worktree it reads the checked-out branch and the repository's common git
directory, the shared `.git` directory that all its worktrees point to, *before* removing
anything. That detail is what lets `rm` work even after a
repository was renamed or removed from `repo_dirs`: the repository and the branch are taken
from the worktree itself, not from the config.

Then the branch. After `git worktree remove` and `git worktree prune`, the branch is deleted
with `git branch -d`, which refuses to delete a branch that is not merged. An unmerged branch
survives with a warning, and the rest of the ticket is removed anyway. A worktree with a
detached HEAD names no branch, so no branch is deleted in that case.

Then the copies. Each `copy` target of the *current* config is removed from the ticket
folder. If a `to` contains a directory, for example `docs/AGENTS.md`, the file is removed
but the empty directory stays; remove it by hand. If the config changed since the ticket was
created, a file that is no longer listed simply stays. Either way the folder is not empty,
and the last step keeps it.

Finally the ticket folder itself, with a plain directory removal rather than a recursive one.
A file you added, a nested repository or anything else `grover` did not create keeps the
folder alive with a warning. This is intentional: `rm` removes the ticket, not the data it
does not know about.

### Failures during removal

A failed step is a warning, not an abort. One dirty worktree does not prevent the next
worktree from being removed, and the command still exits with 0. The cases you are likely to
meet are:

- a dirty worktree (uncommitted files or changes) stays, unless `--force` is given, because
  `git worktree remove` refuses to discard it;
- an unmerged branch stays, unless `--force` is given, because `git branch -d` refuses it;
- a foreign file keeps the ticket folder, even when every worktree and copy is gone.

`--force` removes the worktree with `git worktree remove --force` and the branch with
`git branch -D`, so both of the first two restrictions disappear. It does not make the last
step recursive; unknown files still keep the folder.

### Dry run

`--dry-run` prints the steps it would try — which worktree, which branch, which copy, which
folder — without deleting anything. It still reads each worktree to name the branch and the
repository's common git directory. It is not a full preview: a dirty worktree or an unmerged
branch may survive a real run without `--force`, and files that would keep the ticket folder
are not listed.

## grover config

```text
grover config [-p|--path] [-s|--show] [-i|--init [--force]]
              [--set <field>[=<value>]]
```

Without a flag, `config` opens the interactive editor for the config file. The mode options
`--path`, `--show`, `--init` and `--set` cannot be combined.

| Option | Meaning |
| --- | --- |
| `-p`, `--path` | Print the config file location only |
| `-s`, `--show` | Print the resolved config as YAML |
| `-i`, `--init` | Ask for every value and write a new config |
| `--set FIELD[=VALUE]` | Set one scalar; prompt if `=VALUE` is omitted |
| `--force` | Overwrite an existing file; only with `--init` |

`--path` prints the file that reading would use. Because it needs a location, the file must
exist; a fresh machine with no config gets the usual "no config found" error, which names
the expected path. The output is one line on stdout.

`--show` loads the config and prints it as YAML, with the resolved paths shown in absolute
form and the defaults filled in. It is the mode for scripts and for answering "what is
`grover` actually using?", and it needs no terminal.

`--set FIELD=VALUE` updates one scalar field without opening the editor. Without `=VALUE`,
it prompts on a terminal; an empty value clears `clone_dir`. List fields remain in the
editor. The config file must already exist.

`--init` is the wizard: it numbers its steps (`[2/9] branch_template`), asks for `work_dir`
first and keeps asking until a non-empty answer arrives, then for `branch_template`, `base`,
`create_branch`, `repo_dirs` (a list), and the `repos`, `discover`, `copy` and `setup` lists.
`clone_dir` is not asked; it stays empty until you set it. Existing values are shown as grey
defaults: Enter keeps them, typing replaces them. Esc in a field goes back
to the previous field and keeps what you typed; Esc in the first field aborts. The target is
`--config` when given, else an existing config, else the `dirs` config path, which is
`$XDG_CONFIG_HOME/grover/config.yaml` on Linux. An existing file is refused unless `--force`
is given; with `--force` a loadable file pre-fills every default, and a broken one produces a
warning and starts from the defaults.

The last step is `write <path>?`. Nothing is written before it, so Esc, Ctrl-C or a "no"
there leaves no new file behind and keeps an existing one untouched.

The editor and the wizard both need a terminal. Without one, the editor fails with a hint
to use `--show`, and the wizard fails and names the file it would have written. If no
subcommand is given, `grover` prints its help on stderr and exits with 2, the usual code
for a usage error.

The editor writes after every action, stores the resolved paths as absolute and drops
comments. The wizard writes the values as you typed them, so `~` and relative paths
survive; its file has no comments to lose.
[Configuration](configuration.md#working-with-the-config) explains what that means for a
shared or checked-in config.

## grover completions

```text
grover completions <shell>
```

Prints a completion script for `bash`, `zsh`, `fish`, `elvish` or `powershell` on stdout.
The script completes the subcommands and options; redirect it where your shell reads its
completions:

```console
$ grover completions bash > ~/.local/share/bash-completion/completions/grover
$ grover completions zsh  > ~/.config/zsh/functions/_grover
```

The command does not read the config, so it works before the first ticket exists.

## Environment variables

| Variable | Effect |
| --- | --- |
| `XDG_CONFIG_HOME` | Base for `grover/config.yaml` on Linux; defaults to `~/.config` |
| `HOME` | Source of `~` in config paths and of the default `clone_dir` |
| `GROVER_LOG` | Log filter that overrides `-v` and `-q` |
| `NO_COLOR` | Set to any value to turn off colours in prompts, keeping the layout |
| `GROVER_REPO` | Repo name passed to each setup command |
| `GROVER_TICKET` | Normalized ticket slug passed to each setup command |
| `GIT_TERMINAL_PROMPT` | Set to `0` for `grover`'s own git calls; credentials cannot be typed |

`NO_COLOR` affects the interactive prompts only; the log lines on stderr have no colour.
`GIT_TERMINAL_PROMPT` is part of how `grover` avoids hangs: a private repository without a
usable credential helper fails with a git error rather than waiting for a password.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success, including a run that skipped everything that already existed |
| `1` | The command ran and failed, or a prompt was cancelled |
| `2` | The command line was invalid: unknown option, missing value, bad shell name |

A successful command can still print warnings to stderr; warnings never change the exit
code. That matters in scripts: a `new` that skipped an existing ticket folder is a success,
and a `rm` that could not delete a dirty worktree is still an exit 0. Read stderr, or use
`-q` and look at the code, depending on what you need.
