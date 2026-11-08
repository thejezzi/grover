# Concepts

`grover` is a small tool, but it sits between several things that already have names: a
ticket, a repository, a branch, a git worktree. This page explains the vocabulary in the
order `grover` uses it, so that the [command reference](commands.md) can stay short. Read
this page once and the rest of the documentation reads like a description of something you
already know.

## Tickets and slugs

A ticket is a work item you work on, usually from an issue tracker. Its name is the text
you type, for example `PROJ-123`, `PROJ 123/foo` or `release-2.1`.

Folders and git branch names have stricter rules than issue trackers, so `grover` turns the
ticket name into a slug before using it. The slug keeps ASCII letters, digits, `.` and `_`;
every other character becomes a single `-`; runs of dashes collapse, and leading or
trailing `-` and `.` are trimmed. The result is used for the ticket folder and, through the
branch template, for the branch name:

| Ticket name       | Slug             |
| ----------------- | ---------------- |
| `PROJ-123`        | `PROJ-123`       |
| `PROJ 123/foo`    | `PROJ-123-foo`   |
| `tickets/PROJ-9/` | `tickets-PROJ-9` |
| `  proj 42 `      | `proj-42`        |

This normalization is not cosmetic. Branch names and directory names both reject or
misinterpret characters such as spaces, so `grover` normalizes once and uses the same slug
everywhere. A ticket name with no usable characters left, for instance `-/.`, fails with an
error instead of producing an empty folder name.

## Repositories

A repository is a git repository that a ticket touches. `grover` knows two kinds of
repositories, and both are just entries in the same namespace of repository names.

Local repositories are the child folders of the directories listed in `repo_dirs`. A child
counts as a repository when it contains a `.git` entry, which is true for an ordinary clone
and also for a worktree. The repository name is its folder name, so `~/code/api` is known as
`api`.

Remote repositories come from the `repos` list in the config. Each entry has a URL and an
optional name; without a name, the name is derived from the end of the URL, so
`git@github.com:org/api.git` is called `api`. A remote repository is cloned into
`clone_dir` (default `~/.grover/repos`) the first time a ticket needs it, not before.

Names must be unique within a run. If a local repository and a remote entry share a name,
the local one wins, because local repositories are collected first. Two local repositories
with the same folder name in two `repo_dirs` entries resolve the same way: the first
`repo_dirs` entry wins.

`grover` never fetches or pulls. A local repository is used as it is on disk, and a remote
repository is only cloned when its directory is missing.

## Worktrees and branches

A git worktree is a second working directory for a repository, with its own checked-out
files but the same history and the same branches. The checkout you started from, usually
the one in `repo_dirs`, stays where it is; `grover` adds one worktree per repository inside
the ticket folder, at `<ticket folder>/<repository name>`.

A branch is git's name for a line of commits, and checking one out is what puts files into
a working directory. Each worktree gets a branch. Its name comes from `branch_template`,
where `{ticket}` is replaced by the slug. With the default template `ticket/{ticket}`,
ticket `PROJ-123` uses the branch `ticket/PROJ-123`. Because the branch belongs to the
repository and not to the worktree, it is visible from the original checkout too:
`git branch` lists it, and `git worktree list` shows which folder has it checked out.

If the branch already exists, `grover` reuses it instead of creating a new one. That is what
makes a second `grover new PROJ-123 -r web` add the missing `web` worktree to an existing
ticket without touching the branch that `api` is already using. Git itself refuses to check
out the same branch in two worktrees at once, and `grover` passes that error through rather
than working around it.

Commits made inside a ticket worktree stay on the ticket branch in the shared repository.
`grover` does not commit them, push them or merge them; that is your job or your agent's
job, with ordinary git commands.

## Branches and base refs

A base ref is git's name for the commit a new branch starts from. The config field `base`
holds one or more refs separated by `|`, and `grover` tries them from left to right for each
repository, using the first one that exists there. `HEAD` is the usual last resort: it
always exists in a repository with at least one commit, and means "the commit currently
checked out in the original checkout". A common value is `main|master|HEAD`: `main` in a
repository that has it, `master` in one that does not, and `HEAD` everywhere else.

The base only matters when a branch is new. An existing branch is reused at its current
commit, whatever `base` says. If none of the listed refs exists in a repository, the run
stops with an error that names the refs it tried, because guessing a base would be worse
than failing.

## Plan, apply and dry run

Every `new` runs in two phases. First `grover` builds a plan: it parses the ticket, resolves
the repository selection and computes every target path. This phase writes nothing, so a
mistyped repository name or an ambiguous selection fails while the disk is still untouched.
Then it applies the plan: create the folder, clone where needed, add the worktrees, copy the
files. A base ref is only checked here, when a branch is actually created.

Applying is idempotent. Anything that already exists — the ticket folder, a worktree folder,
a copy target — is skipped with a warning, and the rest of the run continues. Re-running the
same command therefore repairs a partial run and adds newly selected repositories, without
overwriting work that is already in place.

`--dry-run` stops after the plan and prints the same progress lines without writing
anything. The ticket folder path is printed on stdout in a dry run too, so the command
shape does not change between a rehearsal and the real thing.

## How names are matched

Repository names on the command line, and ticket names for commands that only read, are
matched fuzzily and case-insensitively. `grover` tries a fixed ladder of tiers and stops at
the first tier that has a hit:

1. the name is equal to the query,
2. it starts with the query,
3. it contains the query,
4. it is one edit away, where a swapped pair of letters counts as one edit,
5. it is similar enough by Jaro-Winkler similarity, a string metric that rewards matching
   characters near the start of the name.

If a tier has exactly one hit, that name is used. If it has several, the query is ambiguous
and `grover` stops with the list of candidates instead of picking one. If even the closest
name scores below the similarity threshold, the error names the closest names it could find
(up to three).

The point of the ladder is to be generous with typing while staying predictable. `API`
finds `api` at the first tier; `wo` finds `worker` at the second; `wbe` finds `web` at the
fourth. `ap` matches both `api` and `api-gateway` at the second tier, so it is reported as
ambiguous — that is a feature, not a bug.

One command deliberately opts out: `grover rm` resolves a ticket name exactly, because
deleting the wrong ticket is not a mistake a candidate list can fix. Reading commands such
as `grover path` do use the fuzzy match.

## Terminal or script

Anything interactive is gated on the terminal. When stdin is a terminal, `grover` can ask
for a missing ticket, offer a repository picker, and run the config editor. When stdin is not
a terminal — a script, a pipe, a CI job, an agent — it never prompts; it fails with a message
that says what to pass instead. That is why the same command works in both worlds without
hanging.

Prompts are drawn on stderr, not stdout, so the data channel stays clean. This is what makes
`cd "$(grover new ...)"` work even when the command had to ask for a ticket name first.
Cancelling a prompt with Esc or Ctrl-C is an abort: `new` and `rm` have written nothing at
that point and the exit code is 1. The config editor is the exception, because it saves
after every action; there, cancelling keeps what you already confirmed.

In a list, typing filters the entries, the arrow keys move, Enter confirms and Esc cancels.
In a multi-selection, Space toggles the entry under the cursor and the right and left arrows
select all or none. A cancelled prompt never leaves a half-written ticket behind.

## Output and exit codes

`grover` separates data from commentary. Stdout carries the result of the command: the
ticket folder for `new`, the ticket names for `list`, the folder for `path`, YAML for
`config --show`, a path for `config --path`, a script for `completions`. Everything else —
progress messages, warnings, errors — goes to stderr at a level you control with `-q` and
`-v` or the `GROVER_LOG` environment variable.

Exit codes follow the usual convention. `0` means success, including a run that skipped
everything because it was already there. `1` means the command ran and failed, or was
cancelled. `2` means the command line itself was wrong, which clap, the argument parser,
reports before `grover` starts working.

## What grover does not do

Knowing the boundaries is part of understanding the tool.

`grover` creates and removes worktrees and branches; it does not commit, push, fetch, pull,
merge or rebase. It does not change your git config and does not install hooks. It does not
copy anything into the individual repositories: `copy` targets the ticket folder, not the
worktrees inside it.

It also has no hidden state. Everything it knows comes from the config file and from what is
on disk, and every folder it creates carries its own `.git` entry, so a ticket remains a
perfectly ordinary set of git worktrees when you stop using `grover`.

Finally, `grover` never deletes implicitly. Creating is additive and never overwrites;
removing only happens through `grover rm`, which is explicit, exact and documented in the
[command reference](commands.md#grover-rm).
