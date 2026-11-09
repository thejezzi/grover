//! Integration tests against real git repos in a `TempDir`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::git;
use grover_lib::config::Config;
use grover_lib::plan::{Plan, Request};
use tempfile::TempDir;

/// Layout: `<root>/code/api` as local repo, `<root>/templates/AGENTS.md`, and a config
/// with work dir, `repo_dirs` and one `copy` entry.
struct Fixture {
    root: TempDir,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let root = tempfile::tempdir().expect("tempdir");
        let repo = root.path().join("code/api");
        fs::create_dir_all(&repo).expect("repo");
        git(&repo, &["init", "-q"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);

        let template = root.path().join("templates/AGENTS.md");
        fs::create_dir_all(template.parent().expect("parent")).expect("templates");
        fs::write(&template, "# Rules\n").expect("template");

        let config = root.path().join("grover.yaml");
        fs::write(
            &config,
            format!(
                "work_dir: \"{}\"\ncreate_branch: true\nrepo_dirs:\n  - \"{}\"\n\
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

    fn tickets(&self) -> PathBuf {
        self.root.path().join("tickets")
    }

    fn plan(&self, request: Request) -> Plan {
        Plan::build(&self.load(), &request).expect("plan")
    }
}

/// Fixture config plus extra top-level keys, e.g. `base: "main|HEAD"`.
fn config_with(root: &Path, extra: &str) -> Config {
    let path = root.join("custom.yaml");
    fs::write(
        &path,
        format!(
            "work_dir: \"{}\"\ncreate_branch: true\nrepo_dirs: [\"{}\"]\ncopy:\n  - from: \"{}\"\n{extra}",
            root.join("tickets").display(),
            root.join("code").display(),
            root.join("templates/AGENTS.md").display()
        ),
    )
    .expect("config");
    Config::load(Some(&path)).expect("config")
}

fn request(ticket: &str, repos: &[&str]) -> Request {
    Request {
        ticket: ticket.to_string(),
        repos: repos.iter().map(|repo| (*repo).to_string()).collect(),
        all: false,
    }
}

#[test]
fn worktree_branch_and_copy_land_in_ticket_dir() {
    let fixture = Fixture::new();
    let plan = fixture.plan(request("PROJ 123/foo", &["api"]));
    let report = plan.apply(false).expect("apply");

    assert_eq!(plan.branch, "ticket/PROJ-123-foo");
    assert_eq!(plan.ticket_dir, fixture.tickets().join("PROJ-123-foo"));
    assert!(plan.ticket_dir.join("api/.git").exists());
    assert_eq!(fs::read_to_string(plan.ticket_dir.join("AGENTS.md")).expect("copy"), "# Rules\n");
    assert_eq!(
        git(&plan.ticket_dir.join("api"), &["rev-parse", "--abbrev-ref", "HEAD"]),
        plan.branch
    );
    assert_eq!(report.created.len(), 3, "{:?}", report.created);
}

#[test]
fn second_run_is_idempotent() {
    let fixture = Fixture::new();
    let plan = fixture.plan(request("PROJ-1", &["api"]));
    plan.apply(false).expect("first run");
    let report = plan.apply(false).expect("second run");

    assert!(report.created.is_empty(), "{:?}", report.created);
    assert_eq!(report.skipped.len(), 3, "{:?}", report.skipped);
    assert!(plan.ticket_dir.join("AGENTS.md").exists());
}

#[test]
fn dry_run_writes_nothing() {
    let fixture = Fixture::new();
    let plan = fixture.plan(request("PROJ-2", &["api"]));
    let report = plan.apply(true).expect("dry-run");

    assert_eq!(report.created.len(), 3);
    assert!(!plan.ticket_dir.exists());
}

#[test]
fn unknown_name_fails_before_first_write() {
    let fixture = Fixture::new();
    let err =
        Plan::build(&fixture.load(), &request("PROJ-3", &["datenbank"])).expect_err("unknown");
    assert!(err.to_string().contains("similar"), "{err}");
    assert!(!fixture.tickets().exists());
}

#[test]
fn remote_repo_is_cloned_and_checked_out() {
    let fixture = Fixture::new();
    let origin = fixture.root.path().join("origin/web");
    fs::create_dir_all(&origin).expect("origin");
    git(&origin, &["init", "-q"]);
    git(&origin, &["commit", "-q", "--allow-empty", "-m", "init"]);

    let clone_dir = fixture.root.path().join("clones");
    let config = fixture.root.path().join("remote.yaml");
    fs::write(
        &config,
        format!(
            "work_dir: \"{}\"\ncreate_branch: true\nclone_dir: \"{}\"\nrepos:\n  - url: \"{}\"\n",
            fixture.tickets().display(),
            clone_dir.display(),
            origin.display()
        ),
    )
    .expect("config");
    let config = Config::load(Some(&config)).expect("config");

    let plan = Plan::build(&config, &request("PROJ-4", &["web"])).expect("plan");
    plan.apply(false).expect("apply");

    let cloned = clone_dir.join("web");
    assert!(cloned.join(".git").exists());
    assert_eq!(
        git(&plan.ticket_dir.join("web"), &["rev-parse", "--abbrev-ref", "HEAD"]),
        plan.branch
    );
}

#[test]
fn relative_paths_in_config_stay_absolute() {
    let root = tempfile::tempdir().expect("tempdir");
    let project = root.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let path = project.join("grover.yaml");
    fs::write(&path, "work_dir: tickets\nrepo_dirs: [code]\n").expect("config");

    let config = Config::load(Some(&path)).expect("config");
    assert_eq!(config.work_dir, project.join("tickets"));
    assert_eq!(config.repo_dirs, vec![project.join("code")]);
}

#[test]
fn base_refs_fall_back_left_to_right() {
    let fixture = Fixture::new();
    let repo = fixture.root.path().join("code/api");
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "second"]);
    let first = git(&repo, &["rev-list", "--max-parents=0", "HEAD"]);
    git(&repo, &["branch", "stable", &first]);

    let config = config_with(fixture.root.path(), "base: \"missing|stable|HEAD\"\n");
    let plan = Plan::build(&config, &request("PROJ-5", &["api"])).expect("plan");
    plan.apply(false).expect("apply");

    assert_eq!(git(&plan.ticket_dir.join("api"), &["rev-parse", "HEAD"]), first);
}

#[test]
fn without_create_branch_the_worktree_is_detached() {
    let fixture = Fixture::new();
    let path = fixture.root.path().join("detached.yaml");
    fs::write(
        &path,
        format!(
            "work_dir: \"{}\"\nrepo_dirs: [\"{}\"]\n",
            fixture.tickets().display(),
            fixture.root.path().join("code").display()
        ),
    )
    .expect("config");
    let config = Config::load(Some(&path)).expect("config");
    let plan = Plan::build(&config, &request("PROJ-9", &["api"])).expect("plan");
    plan.apply(false).expect("apply");

    let worktree = plan.ticket_dir.join("api");
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "HEAD",
        "detached, no ticket branch"
    );
    let branches = git(&fixture.root.path().join("code/api"), &["branch", "--list"]);
    assert!(!branches.contains("ticket/PROJ-9"), "{branches}");
}

#[test]
fn create_branch_true_keeps_the_old_behaviour() {
    let fixture = Fixture::new();
    let plan = fixture.plan(request("PROJ-10", &["api"]));
    plan.apply(false).expect("apply");
    assert!(plan.create_branch);
    assert_eq!(
        git(&plan.ticket_dir.join("api"), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "ticket/PROJ-10"
    );
}

#[test]
fn missing_base_refs_fail_with_their_names() {
    let fixture = Fixture::new();
    let config = config_with(fixture.root.path(), "base: \"nope|also-nope\"\n");
    let plan = Plan::build(&config, &request("PROJ-6", &["api"])).expect("plan");
    let err = plan.apply(false).expect_err("no base ref");
    assert!(err.to_string().contains("nope"), "{err}");
    assert!(err.to_string().contains("also-nope"), "{err}");
}
