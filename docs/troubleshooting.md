# Troubleshooting

`grover` reports runtime problems on stderr and exits with 1. The message is usually one
sentence that names the thing that went wrong — a path, a name or a git ref — so the fastest
way to use this page is to read the message first, then find its section here. When a command
fails before doing any work, which most validation errors do, the disk is unchanged; when a
command fails halfway through, the progress lines show how far it got, and running the same
command again resumes rather than repeats. (A command line that does not parse exits with 2
before any of this; see [Exit codes](commands.md#exit-codes).)

Two flags help with diagnosis. `-v` adds debug output, and `-v -v` adds trace output; both
go to stderr, so they never mix into a captured path. `-q` does the opposite and hides
progress, which makes a failure easier to spot in a long run.

The vocabulary used here — ticket, worktree, branch, base ref — is defined in
[Concepts](concepts.md). That page also explains why some commands prompt and others fail.

## The config cannot be found

```text
no config found, expected: /home/you/.config/grover/config.yaml
```

`grover` searched `--config`, then `./grover.yaml`, then `$XDG_CONFIG_HOME/grover/config.yaml`
and found nothing. Run `grover config --init` on a terminal to create the last one, write a
`grover.yaml` yourself, or pass an explicit `--config`. With `--config`, the message names
the path that was requested, so a relative path that resolved somewhere unexpected is
visible immediately.

The same error appears for `grover config --path`, which reports a location and therefore
needs an existing file. On a fresh machine, `--init` is the command you want, not `--path`.

## The config is not accepted

A file that exists but does not parse produces the path and the parser's explanation:

```text
/home/you/.config/grover/config.yaml: unknown field `repo_dir`, expected one of
`work_dir`, `branch_template`, `create_branch`, `base`, `repo_dirs`, `clone_dir`,
`repos`, `discover`, `copy`, `setup`
at line 2 column 1
```

Unknown field names are errors by design; a typo such as `repo_dir` does not silently behave
like an empty list. Check the spelling against [Configuration](configuration.md#fields), and
keep in mind that the key must be at the right level: `url` belongs inside a `repos` entry,
not next to `repos`.

A file that parses but does not make sense fails with `invalid config: ...` followed by the
reason. The common ones are a `branch_template` without `{ticket}`, a `base` with no ref, a
`work_dir` that is empty, a config with neither `repo_dirs`, `repos` nor `discover`, a `copy`
entry with an empty `from`, and a `copy` entry whose `to` is absolute. Fix the named field;
nothing has been written.

## A command wants a terminal

```text
no terminal to ask for the values; write /home/you/.config/grover/config.yaml yourself
no terminal for the editor; use `grover config --show`
missing ticket: pass <ticket>
```

These messages mean the command wanted to ask something and stdin is not a terminal, so it
refused to wait. That is the intended behavior in scripts, CI and agents: a prompt there
would hang forever. Pass the value instead — a ticket name, `--repo` or `--all` — or use the
non-interactive variant, `grover config --show` instead of the editor and a hand-written
file instead of `--init`.

`grover rm` with no argument adds one more case: it does not look at the current directory to
guess a ticket. Give it a name, or run it on a terminal and pick from the list.

## A ticket name is not usable

```text
ticket " -/. " yields no directory name
```

The slug rule kept no letters, digits, `.` or `_`, so there is no folder name to use. This is
the same normalization that turns spaces and slashes into `-`; the name needs at least one
character that survives it. Use a ticket name that contains letters or digits.

## There is no ticket to pick

```text
no tickets in /home/you/work/tickets
```

`grover rm` without an argument opens the picker on a terminal, and the picker has nothing to
show. `work_dir` is empty or does not exist yet. Create a ticket with `grover new`, or pass a
name directly.

## A repository name does not match

```text
unknown name "datenbank"; similar: api, api-gateway, db
```

No repository came close enough. The suggestions are the nearest names `grover` knows (up to
three). Check `repo_dirs` and `repos` in the config, and remember that only direct children
with a `.git` entry are discovered — a repository nested one level deeper is invisible.

```text
ambiguous name "ap", candidates: api, api-gateway
```

The query fits several names equally well at the same matching tier, so `grover` stops
instead of choosing. Type more of the name. `--all` sidesteps matching entirely.

```text
no repo selected: pass `--repo <name>` or `--all`
no repos found: check `repo_dirs`, `repos` and `discover`
```

The first means no repository was selected and stdin is not a terminal. The second means the
selection machinery found no repositories at all: the configured folders contain no children
with a `.git` entry, `repos` is empty, and no `discover` command printed a usable URL. A
`repo_dirs` folder that does not exist produces a warning; if you see no warning and still no
repositories, the folder is there but none of its children is a git repository.

A `discover` command that fails is only a warning, so this error is also what you see when
the only source was a `discover` entry and it failed: `gh` not installed, not logged in, or
the org name wrong. The warning above the error carries the command's own stderr. A
`discover` that printed URLs but no repositories at all means `clone_dir` is missing — the
warning then says `repos`/`discover` are ignored.

## A base ref does not exist

```text
repo /home/you/code/api: none of the base refs "nope|also-nope" exists
```

The `base` list was tried from left to right in this repository and none of the refs
resolved. Either fix the ref names, or set `base` to something that exists everywhere, for
example `main|master|HEAD`. The same message appears with `HEAD` for a repository that has
no commits yet: a fresh `git init` has no `HEAD` to start from, so make one commit first.

An existing ticket branch does not consult `base` at all, so this error only concerns
branches that are about to be created.

## git fails

`grover` passes git's own message through after a prefix that describes what it was doing:

```text
git worktree add ticket/PROJ-9 in /work/tickets/PROJ-9/api: <git's message>
```

Read the git message as you would in the terminal; it is verbatim, including its language.
The most common causes are:

- **The branch is checked out somewhere else.** Git refuses to attach one branch to two
  worktrees. Run `git worktree list` in the repository to see where it is, and either remove
  that worktree or pick another ticket name.
- **The repository has no commits.** See the base ref section above.
- **The clone needs credentials.** `grover` sets `GIT_TERMINAL_PROMPT=0`, so a private
  repository without a working credential helper fails instead of asking for a password.
  Configure an ssh key or a credential helper, then run the command again.
- **The destination is not a repository.** A folder named like a remote repository can
  already exist in `clone_dir` without being a valid clone; `git clone` refuses, and the
  message says so.

A failure in the middle of `new` leaves the folders created so far in place. Fix the cause
and run the same command again: existing folders are skipped, and the missing ones are
created.

If git itself cannot be started, the message is `cannot run git: ...`; check that `git` is on
`PATH`. Any other file system failure is reported as `<path>: <reason>`, which is the usual
form for a copy source that does not exist or a directory that cannot be written.

## Everything was skipped

```text
WARN /work/tickets/PROJ-123 already exists
WARN /work/tickets/PROJ-123/api already exists
INFO done created=0 skipped=3 cloned=0
```

This is success, not a failure. `new` is idempotent: it never overwrites, and an existing
ticket folder, worktree or copy target is skipped with a warning. The ticket folder is still
printed on stdout, so `cd "$(grover new ...)"` keeps working. Re-running is how you add a
repository to an existing ticket or repair a run that stopped early.

Use `--dry-run` if you want to see what would be created before committing to it, and `-q`
if the warnings in a script are just noise.

## Removing a ticket leaves things behind

`rm` is conservative on purpose. A dirty worktree, an unmerged branch or an unknown file is
kept with a warning, and the command still exits with 0, because it removed everything it
was allowed to remove.

A dirty worktree is one with uncommitted changes. Commit or discard them, or re-run with
`--force` if you are sure they are disposable. An unmerged branch is one whose commits are
not in the branch git compares against; merge it, or use `--force` to delete it with
`git branch -D`. Neither case makes `rm` recursive.

The ticket folder survives while anything remains inside it. Often that is a foreign file:
a notes file, a second nested repository, or a file that a previous config listed under
`copy` but the current one does not. A dirty worktree keeps it too, as described above. An
unmerged branch does not: the branch stays in the original repository, but the ticket folder
can be removed. Delete the foreign files yourself and run `rm` again, or remove the folder
by hand once you have checked it.

A detached HEAD in a worktree is removed without deleting any branch, because there is no
branch to name. A repository that has since disappeared from `repo_dirs` is not a problem:
`rm` takes the repository and the branch from the worktree itself, so it still cleans up.

## grover path says the directory is not in work_dir

```text
no ticket folder above /home/you/work in /home/you/work/tickets
```

Without an argument, `path` infers the ticket from the current directory, which must be
inside `work_dir` or below it. Running it from `work_dir` itself is not enough, because there
is no first path component to take. Either `cd` into a ticket or pass the ticket name.

The comparison uses the paths as written, without resolving symbolic links (symlinks). If
`work_dir` is reached through a symlink, the no-argument form can disagree with the config
even though the folder looks right. Pass the ticket name, or use the real path, and the
ambiguity disappears.

## The editor changed my config

The interactive editor writes the config after every action, stores the resolved paths as
absolute and re-renders the file from the parsed values, so comments disappear on the first
save. That is by design: preserving comments while editing structured values needs a
round-tripping YAML editor, which is more machinery than the feature deserves. The wizard
does not resolve paths, so a `~` or a relative path you typed there stays as you typed it;
it only writes a fresh file, which has no comments to lose.

If the config is a symlink, the link is kept and the file it points to is written, so a
dotfiles setup keeps working. If a new value fails validation, the previous file is restored
and the problem is printed as a warning; the menu stays open, and leaving it keeps the last
good state. For a config that is shared or checked in, edit it by hand and use the editor
only for quick changes.

## Nothing is in the list

`grover list` prints nothing, without an error, when `work_dir` does not exist. That is the
normal output on a fresh setup before the first ticket. If you expected tickets, check
`grover config --show` for the resolved `work_dir`; a relative path in the config is
resolved against the config file, not against the current directory, so the two can differ
more than you expect.

Only directories are listed. If `work_dir` contains files that look like ticket names, they
will not appear; `grover` treats them as foreign.

## Reading further

Sometimes the fastest answer is to ask git directly. `git -C <worktree> status` shows what
makes a worktree dirty, `git worktree list` shows every worktree of a repository and the
branch attached to it, and `git branch -vv` shows each branch's upstream and how far it has
diverged. In the ticket folder, `grover rm --dry-run <ticket>` lists what a removal would
try to remove. It does not list foreign files, and a dirty worktree or an unmerged branch may
still survive, so also look at the folder itself.
