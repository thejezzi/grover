//! CLI integration tests: the binary with a closed stdin, so there is never a TTY.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use grover_lib::config::Config;
use tempfile::TempDir;

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grover"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("grover starts")
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=grover", "-c", "user.email=grover@example.com"])
        .args(args)
        .output()
        .expect("git starts");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

/// Config with a work dir and one repo dir; `code/api` exists as a real repo.
fn fixture() -> (TempDir, String) {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = root.path().join("code/api");
    fs::create_dir_all(&repo).expect("repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);

    let path = root.path().join("grover.yaml");
    let yaml = format!(
        "work_dir: \"{}\"\nrepo_dirs: [\"{}\"]\n",
        root.path().join("tickets").display(),
        root.path().join("code").display()
    );
    fs::write(&path, yaml).expect("config");
    let path = path.to_str().expect("utf8").to_string();
    (root, path)
}

#[test]
fn init_writes_the_example_config_once() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("conf/config.yaml");
    let config = path.to_str().expect("utf8");

    let first = run(root.path(), &["init", "-c", config]);
    assert_eq!(first.status.code(), Some(0), "{}", err(&first));
    assert_eq!(out(&first), "", "stdout stays empty");
    Config::load(Some(&path)).expect("written config loads");

    let second = run(root.path(), &["init", "-c", config]);
    assert_eq!(second.status.code(), Some(1));
    assert!(err(&second).contains("--force"), "{}", err(&second));

    let forced = run(root.path(), &["init", "-c", config, "--force"]);
    assert_eq!(forced.status.code(), Some(0), "{}", err(&forced));
}

#[test]
fn init_uses_the_grover_xdg_config_dir() {
    let root = tempfile::tempdir().expect("tempdir");
    let init = Command::new(env!("CARGO_BIN_EXE_grover"))
        .arg("init")
        .current_dir(root.path())
        .env("XDG_CONFIG_HOME", root.path().join("xdg"))
        .stdin(Stdio::null())
        .output()
        .expect("grover starts");

    assert_eq!(init.status.code(), Some(0), "{}", err(&init));
    let path = root.path().join("xdg/grover/config.yaml");
    assert!(path.exists());
    Config::load(Some(&path)).expect("written config loads");
}

#[test]
fn config_shows_the_yaml_and_the_path() {
    let (root, config) = fixture();

    let shown = run(root.path(), &["config", "-c", &config, "--show"]);
    assert_eq!(shown.status.code(), Some(0), "{}", err(&shown));
    assert!(out(&shown).contains("work_dir:"), "{}", out(&shown));

    let located = run(root.path(), &["config", "-c", &config, "--path"]);
    assert_eq!(located.status.code(), Some(0), "{}", err(&located));
    assert!(out(&located).ends_with("grover.yaml"), "{}", out(&located));
}

#[test]
fn config_editor_without_a_terminal_points_to_show() {
    let (root, config) = fixture();
    let editor = run(root.path(), &["config", "-c", &config]);
    assert_eq!(editor.status.code(), Some(1));
    assert!(err(&editor).contains("--show"), "{}", err(&editor));
}

#[test]
fn new_without_a_terminal_refuses_to_prompt() {
    let (root, config) = fixture();

    let no_ticket = run(root.path(), &["new", "-c", &config]);
    assert_eq!(no_ticket.status.code(), Some(1));
    assert!(err(&no_ticket).contains("missing ticket"), "{}", err(&no_ticket));

    let no_repo = run(root.path(), &["new", "PROJ-1", "-c", &config]);
    assert_eq!(no_repo.status.code(), Some(1));
    assert!(err(&no_repo).contains("no repo selected"), "{}", err(&no_repo));
    assert!(!root.path().join("tickets").exists(), "nothing written");
}

#[test]
fn new_prints_only_the_ticket_dir() {
    let (root, config) = fixture();
    let created = run(root.path(), &["new", "PROJ-1", "-r", "api", "-c", &config]);

    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    let ticket = root.path().join("tickets/PROJ-1");
    assert_eq!(out(&created), ticket.display().to_string());
    assert!(ticket.join("api/.git").exists());
}

#[test]
fn list_prints_only_existing_ticket_names() {
    let (root, config) = fixture();
    for ticket in ["PROJ-2", "PROJ-1"] {
        let created = run(root.path(), &["new", ticket, "-r", "api", "-c", &config]);
        assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    }
    fs::write(root.path().join("tickets/notes.txt"), "not a work item").expect("decoy");

    let listed = run(root.path(), &["list", "-c", &config]);
    assert_eq!(listed.status.code(), Some(0), "{}", err(&listed));
    assert_eq!(
        out(&listed),
        "PROJ-1
PROJ-2"
    );
}

#[test]
fn list_without_a_work_dir_is_empty() {
    let (root, config) = fixture();
    let listed = run(root.path(), &["list", "-c", &config]);

    assert_eq!(listed.status.code(), Some(0), "{}", err(&listed));
    assert_eq!(out(&listed), "");
    assert!(!root.path().join("tickets").exists());
}
