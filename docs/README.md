# grover documentation

The [README](../README.md) is the short version: what `grover` is, how to install it, and
what a first ticket looks like. This folder is the long version. It explains the ideas the
commands are built on, documents every option and config field, and tells you what to do
when a command refuses to run.

If you are new, read [Getting started](getting-started.md) top to bottom and keep the
terminal open; it walks you to a ticket folder you can work in. If you only want to
understand how the tool behaves, read [Concepts](concepts.md).

- [Getting started](getting-started.md) takes you from installation to your first ticket,
  then to the daily loop of creating, finding and removing tickets.
- [Concepts](concepts.md) explains the vocabulary: tickets and their slugs, repositories,
  worktrees, branches, base refs, fuzzy matching, and why some commands ask questions
  while others fail.
- [Configuration](configuration.md) documents the config file: where it lives, how paths
  are resolved, every field, and the interactive editor.
- [Commands](commands.md) is the complete command-line reference: every command, every
  option, the environment variables and the exit codes.
- [Troubleshooting](troubleshooting.md) lists the error messages you are likely to see and
  the way out of each one.

The design decision log is [PLAN.md](../PLAN.md), and the repository rules for changing
`grover` are in [AGENTS.md](../AGENTS.md). Neither is needed to use the tool.
