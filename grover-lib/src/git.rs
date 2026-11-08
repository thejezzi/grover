//! Thin wrapper around the `git` CLI: no `git2`, same behaviour as the terminal.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use crate::error::{Error, Result};

/// `.git` as directory or file: worktrees carry a file.
pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

pub fn branch_exists(repo: &Path, branch: &str) -> Result<bool> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"));
    succeeded(cmd)
}

/// Add a worktree; an existing branch is reused, else the first existing base ref.
pub fn worktree_add(repo: &Path, dir: &Path, branch: &str, bases: &str) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).args(["worktree", "add"]);
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
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(repo).args(["rev-parse", "--verify", "--quiet"]).arg(base);
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

/// Without prompt: missing credentials should fail, not hang.
fn output(mut cmd: Command) -> Result<Output> {
    cmd.env("GIT_TERMINAL_PROMPT", "0").output().map_err(Error::Spawn)
}

fn succeeded(cmd: Command) -> Result<bool> {
    Ok(output(cmd)?.status.success())
}

fn run(cmd: Command, command: String) -> Result<()> {
    let out = output(cmd)?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(Error::Git { command, stderr })
}
