//! Thin wrapper around the `git` CLI: no `git2`, same behaviour as the terminal.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::error::{Error, Result};

/// `.git` as directory or file: worktrees carry a file.
pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

/// `git` with `-C`: every command here works inside a repo.
fn git(repo: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo);
    cmd
}

fn branch_exists(repo: &Path, branch: &str) -> Result<bool> {
    let mut cmd = git(repo);
    cmd.args(["show-ref", "--verify", "--quiet"]).arg(format!("refs/heads/{branch}"));
    succeeded(cmd)
}

/// Add a worktree; an existing branch is reused, else the first existing base ref.
pub fn worktree_add(repo: &Path, dir: &Path, branch: &str, bases: &str) -> Result<()> {
    let mut cmd = git(repo);
    cmd.args(["worktree", "add"]);
    if branch_exists(repo, branch)? {
        cmd.arg(dir).arg(branch);
    } else {
        cmd.args(["-b", branch]).arg(dir).arg(first_existing(repo, bases)?);
    }
    run(cmd, format!("worktree add {branch} in {}", dir.display()))
}

/// First ref of `main|master|HEAD` that resolves in this repo.
fn first_existing<'a>(repo: &Path, bases: &'a str) -> Result<&'a str> {
    for base in bases.split('|').map(str::trim).filter(|base| !base.is_empty()) {
        let mut cmd = git(repo);
        cmd.args(["rev-parse", "--verify", "--quiet"]).arg(base);
        if succeeded(cmd)? {
            return Ok(base);
        }
    }
    Err(Error::BaseMissing { repo: repo.to_path_buf(), bases: bases.to_string() })
}

pub fn clone(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io { path: parent.into(), source })?;
    }
    let mut cmd = Command::new("git");
    cmd.args(["clone", "--quiet", url]).arg(dest);
    run(cmd, format!("clone {url} to {}", dest.display()))
}

/// Branch checked out in this worktree; `None` when HEAD is detached.
pub fn head_branch(worktree: &Path) -> Result<Option<String>> {
    let mut cmd = git(worktree);
    cmd.args(["rev-parse", "--abbrev-ref", "HEAD"]);
    let branch = stdout(cmd, format!("branch of {}", worktree.display()))?;
    Ok((branch != "HEAD").then_some(branch))
}

/// Common git dir of a worktree, so `rm` needs no repo from the config.
pub fn common_dir(worktree: &Path) -> Result<PathBuf> {
    let mut cmd = git(worktree);
    cmd.args(["rev-parse", "--git-common-dir"]);
    let dir = stdout(cmd, format!("common dir of {}", worktree.display()))?;
    // git reports it relative to the worktree.
    Ok(worktree.join(dir))
}

/// Remove a worktree; `force` drops uncommitted changes.
pub fn worktree_remove(repo: &Path, worktree: &Path, force: bool) -> Result<()> {
    let mut cmd = git(repo);
    cmd.args(["worktree", "remove"]);
    if force {
        cmd.arg("--force");
    }
    cmd.arg(worktree);
    run(cmd, format!("worktree remove {}", worktree.display()))
}

/// Drop worktree entries whose directory is gone.
pub fn worktree_prune(repo: &Path) -> Result<()> {
    let mut cmd = git(repo);
    cmd.args(["worktree", "prune"]);
    run(cmd, format!("worktree prune in {}", repo.display()))
}

/// Delete a branch in `repo`; `-d` keeps unmerged branches, `force` uses `-D`.
pub fn branch_delete(repo: &Path, branch: &str, force: bool) -> Result<()> {
    let mut cmd = git(repo);
    cmd.args(["branch"]).arg(if force { "-D" } else { "-d" }).arg(branch);
    run(cmd, format!("branch delete {branch} in {}", repo.display()))
}

/// Without prompt: missing credentials should fail, not hang.
fn output(mut cmd: Command) -> Result<Output> {
    cmd.env("GIT_TERMINAL_PROMPT", "0").output().map_err(Error::Spawn)
}

fn succeeded(cmd: Command) -> Result<bool> {
    Ok(output(cmd)?.status.success())
}

fn run(cmd: Command, command: String) -> Result<()> {
    stdout(cmd, command).map(|_| ())
}

/// Like `run`, but hands back stdout.
fn stdout(cmd: Command, command: String) -> Result<String> {
    let out = output(cmd)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(Error::Git { command, stderr })
}
