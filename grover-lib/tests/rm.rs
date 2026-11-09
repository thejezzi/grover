//! Integration tests for `rm` against real git repos in a `TempDir`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::git;
use grover_lib::config::Config;
use grover_lib::plan::{self, Plan, Request};
use tempfile::TempDir;

/// Layout: `<root>/code/api` as local repo, a template with one `copy` entry, a config.
struct Fixture {
    root: TempDir,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::tempdir().expect("tempdir");
        add_repo(&root.path().join("code"), "api");

        let template = root.path().join("templates/AGENTS.md");
        fs::create_dir_all(template.parent().expect("parent")).expect("templates");
        fs::write(&template, "# Rules\n").expect("template");

        let config = root.path().join("grover.yaml");
        fs::write(
            &config,
            format!(
                "work_dir: \"{}\"\ncreate_branch: true\nrepo_dirs: [\"{}\"]\n\
                 copy:\n  - from: \"{}\"\n    to: AGENTS.md\n",
                root.path().join("tickets").display(),
                root.path().join("code").display(),
                template.display()
            ),
        )
        .expect("config");
        Fixture { root, config }
    }

    fn load(&self) -> Config {
        Config::load(Some(&self.config)).expect("config")
    }

    fn repo(&self, name: &str) -> PathBuf {
        self.root.path().join("code").join(name)
    }

    /// A ticket created like `new` does: worktree, branch, `copy` file.
    fn ticket(&self, name: &str, repos: &[&str]) -> PathBuf {
        let request = Request {
            ticket: name.to_string(),
            repos: repos.iter().map(|repo| (*repo).to_string()).collect(),
            all: false,
        };
        let plan = Plan::build(&self.load(), &request).expect("plan");
        plan.apply(false).expect("apply");
        plan.ticket_dir
    }
}

fn add_repo(code: &Path, name: &str) -> PathBuf {
    let repo = code.join(name);
    fs::create_dir_all(&repo).expect("repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    repo
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .status()
        .expect("git starts")
        .success()
}

#[test]
fn worktrees_branches_copies_and_dir_are_gone() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-1", &["api"]);
    let repo = fixture.repo("api");

    plan::remove(&fixture.load(), &dir, false, false).expect("remove");

    assert!(!dir.exists(), "the copy file had to go for the dir to go");
    assert!(!branch_exists(&repo, "ticket/PROJ-1"), "merged branch is deleted");
    let listed = git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(!listed.contains(&dir.join("api").display().to_string()), "{listed}");
}

#[test]
fn a_config_without_the_repo_still_removes() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-8", &["api"]);
    let config = fixture.root.path().join("elsewhere.yaml");
    fs::write(
        &config,
        format!(
            "work_dir: \"{}\"\nrepo_dirs: [\"elsewhere\"]\n\
             copy:\n  - from: \"{}\"\n    to: AGENTS.md\n",
            fixture.root.path().join("tickets").display(),
            fixture.root.path().join("templates/AGENTS.md").display()
        ),
    )
    .expect("config");

    let without_repo = Config::load(Some(&config)).expect("config");
    plan::remove(&without_repo, &dir, false, false).expect("remove");

    assert!(!dir.exists());
    assert!(!branch_exists(&fixture.repo("api"), "ticket/PROJ-8"));
}

#[test]
fn dirty_worktree_stays_without_force_but_the_rest_goes() {
    let fixture = Fixture::new();
    add_repo(&fixture.root.path().join("code"), "web");
    let dir = fixture.ticket("PROJ-2", &["api", "web"]);
    fs::write(dir.join("api/notes.txt"), "uncommitted").expect("dirty");

    plan::remove(&fixture.load(), &dir, false, false).expect("remove");

    assert!(dir.join("api").exists(), "dirty worktree kept");
    assert!(!dir.join("web").exists(), "the next repo is still removed");
    assert!(dir.exists(), "the kept worktree holds the dir");
}

#[test]
fn unmerged_branch_stays_without_force() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-3", &["api"]);
    git(&dir.join("api"), &["commit", "-q", "--allow-empty", "-m", "work"]);

    plan::remove(&fixture.load(), &dir, false, false).expect("remove");

    assert!(branch_exists(&fixture.repo("api"), "ticket/PROJ-3"), "unmerged branch kept");
    assert!(!dir.join("api").exists(), "worktree gone");
}

#[test]
fn force_removes_a_dirty_worktree_and_its_unmerged_branch() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-4", &["api"]);
    git(&dir.join("api"), &["commit", "-q", "--allow-empty", "-m", "work"]);
    fs::write(dir.join("api/notes.txt"), "uncommitted").expect("dirty");

    plan::remove(&fixture.load(), &dir, true, false).expect("force");

    assert!(!dir.exists());
    assert!(!branch_exists(&fixture.repo("api"), "ticket/PROJ-4"));
}

#[test]
fn a_detached_worktree_is_removed_without_deleting_a_branch() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-10", &["api"]);
    git(&dir.join("api"), &["checkout", "-q", "--detach"]);

    plan::remove(&fixture.load(), &dir, false, false).expect("remove");

    assert!(!dir.exists());
    // A detached HEAD names no branch, so there is nothing to delete.
    assert!(branch_exists(&fixture.repo("api"), "ticket/PROJ-10"));
}

#[test]
fn a_foreign_file_keeps_the_ticket_dir() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-5", &["api"]);
    fs::write(dir.join("notes.md"), "mine").expect("notes");

    plan::remove(&fixture.load(), &dir, false, false).expect("remove");

    assert!(dir.exists());
    assert!(dir.join("notes.md").exists());
    assert!(!dir.join("AGENTS.md").exists(), "the copy goes");
    assert!(!dir.join("api").exists());
}

#[test]
fn dry_run_removes_nothing() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-6", &["api"]);

    plan::remove(&fixture.load(), &dir, false, true).expect("dry-run");

    assert!(dir.join("api/.git").exists());
    assert!(dir.join("AGENTS.md").exists());
    assert!(branch_exists(&fixture.repo("api"), "ticket/PROJ-6"));
}

#[test]
fn unknown_ticket_changes_nothing() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-7", &["api"]);

    let err = plan::resolve_ticket(&fixture.load(), "PROJ-8").expect_err("unknown");

    assert!(err.to_string().contains("similar"), "{err}");
    assert!(dir.join("api").exists());
    assert!(branch_exists(&fixture.repo("api"), "ticket/PROJ-7"));
}

#[test]
fn resolve_never_guesses_a_nearby_ticket() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-12", &["api"]);

    // A prefix match would hit PROJ-12 here; deleting must stay exact.
    assert!(plan::resolve_ticket(&fixture.load(), "PROJ-1").is_err());
    assert_eq!(plan::resolve_ticket(&fixture.load(), "PROJ-12").expect("exact"), dir);
}
