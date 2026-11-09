//! Integration tests for `status` against real git repos in a `TempDir`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::git;
use grover_lib::config::Config;
use grover_lib::plan::{self, Plan, Request};
use tempfile::TempDir;

/// Layout: `<root>/code` with repos, a config with a work dir and one `repo_dir`.
struct Fixture {
    root: TempDir,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::tempdir().expect("tempdir");
        add_repo(&root.path().join("code"), "api");

        let config = root.path().join("grover.yaml");
        fs::write(
            &config,
            format!(
                "work_dir: \"{}\"\ncreate_branch: true\nrepo_dirs: [\"{}\"]\n",
                root.path().join("tickets").display(),
                root.path().join("code").display()
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

    /// A ticket created like `new` does: one worktree per repository.
    fn ticket(&self, name: &str, repos: &[&str]) -> PathBuf {
        let request = Request {
            ticket: name.to_string(),
            repos: repos.iter().map(|repo| (*repo).to_string()).collect(),
            all: false,
            ..Request::default()
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

#[test]
fn a_clean_worktree_without_an_upstream_is_clean() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-1", &["api"]);

    let statuses = plan::statuses(&dir).expect("status");

    assert_eq!(statuses.len(), 1);
    let status = &statuses[0];
    assert_eq!(status.name, "api");
    assert_eq!(status.branch.as_deref(), Some("ticket/PROJ-1"));
    assert_eq!(status.ahead, None, "a fresh ticket branch has no upstream");
    assert_eq!(status.behind, None);
    assert_eq!(status.dirty, 0);
}

#[test]
fn dirty_files_are_counted() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-2", &["api"]);
    fs::write(dir.join("api/notes.txt"), "uncommitted").expect("dirty");
    fs::write(dir.join("api/other.txt"), "uncommitted").expect("dirty");

    let statuses = plan::statuses(&dir).expect("status");

    assert_eq!(statuses[0].dirty, 2);
}

#[test]
fn ahead_and_behind_are_counted_against_the_upstream() {
    let fixture = Fixture::new();
    let repo = fixture.repo("api");
    let dir = fixture.ticket("PROJ-3", &["api"]);
    let worktree = dir.join("api");
    let base = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
    git(&worktree, &["branch", "--set-upstream-to", &base]);

    git(&worktree, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
    let statuses = plan::statuses(&dir).expect("status");
    assert_eq!((statuses[0].ahead, statuses[0].behind), (Some(1), Some(0)));

    git(&repo, &["commit", "-q", "--allow-empty", "-m", "behind"]);
    let statuses = plan::statuses(&dir).expect("status");
    assert_eq!((statuses[0].ahead, statuses[0].behind), (Some(1), Some(1)));
}

#[test]
fn foreign_entries_in_the_ticket_dir_are_ignored() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-4", &["api"]);
    fs::write(dir.join("notes.txt"), "not a repo").expect("notes");

    let statuses = plan::statuses(&dir).expect("status");

    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].name, "api");
}

#[test]
fn a_detached_worktree_has_no_branch() {
    let fixture = Fixture::new();
    let dir = fixture.ticket("PROJ-5", &["api"]);
    git(&dir.join("api"), &["checkout", "-q", "--detach"]);

    let statuses = plan::statuses(&dir).expect("status");

    assert_eq!(statuses[0].branch, None);
}

#[test]
fn worktrees_are_sorted_by_name() {
    let fixture = Fixture::new();
    add_repo(&fixture.root.path().join("code"), "web");
    let dir = fixture.ticket("PROJ-6", &["web", "api"]);

    let statuses = plan::statuses(&dir).expect("status");

    let names: Vec<&str> = statuses.iter().map(|status| status.name.as_str()).collect();
    assert_eq!(names, ["api", "web"]);
}
