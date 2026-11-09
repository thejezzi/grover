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
    git_out(dir, args);
}

/// `git` that returns trimmed stdout; panics on a failing command.
fn git_out(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=grover", "-c", "user.email=grover@example.com"])
        .args(args)
        .output()
        .expect("git starts");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Config with a work dir and one repo dir; `code/api` exists as a real repo.
fn fixture() -> (TempDir, String) {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = root.path().join("code/api");
    fs::create_dir_all(&repo).expect("repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);

    let path = root.path().join("grover.yaml");
    let work_dir = root.path().canonicalize().expect("root").join("tickets");
    let yaml = format!(
        "work_dir: \"{}\"\ncreate_branch: true\nrepo_dirs: [\"{}\"]\n",
        work_dir.display(),
        root.path().join("code").display()
    );
    fs::write(&path, yaml).expect("config");
    let path = path.to_str().expect("utf8").to_string();
    (root, path)
}

#[test]
fn config_init_without_a_terminal_writes_nothing() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("conf/config.yaml");

    let refused = run(root.path(), &["config", "--init", "-c", path.to_str().expect("utf8")]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(err(&refused).contains("no terminal"), "{}", err(&refused));
    assert_eq!(out(&refused), "", "stdout stays empty");
    assert!(!path.exists(), "nothing written");
}

#[test]
fn config_init_needs_force_for_an_existing_file() {
    let (root, config) = fixture();
    let before = fs::read_to_string(&config).expect("read");

    let refused = run(root.path(), &["config", "--init", "-c", &config]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(err(&refused).contains("--force"), "{}", err(&refused));

    let forced = run(root.path(), &["config", "--init", "--force", "-c", &config]);
    assert_eq!(forced.status.code(), Some(1), "no terminal either way");
    assert!(err(&forced).contains("no terminal"), "{}", err(&forced));
    assert_eq!(fs::read_to_string(&config).expect("read"), before, "file kept");
}

#[test]
fn config_init_uses_the_platform_config_path() {
    let root = tempfile::tempdir().expect("tempdir");
    let xdg = root.path().join("xdg");
    let appdata = root.path().join("AppData/Roaming");
    let path = if cfg!(target_os = "macos") {
        root.path().join("Library/Application Support/grover/config.yaml")
    } else if cfg!(target_os = "windows") {
        appdata.join("grover/config.yaml")
    } else {
        xdg.join("grover/config.yaml")
    };
    let init = Command::new(env!("CARGO_BIN_EXE_grover"))
        .args(["config", "--init"])
        .current_dir(root.path())
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", xdg)
        .env("APPDATA", appdata)
        .stdin(Stdio::null())
        .output()
        .expect("grover starts");

    assert_eq!(init.status.code(), Some(1));
    assert!(err(&init).contains(&path.display().to_string()), "{}", err(&init));
    assert!(!path.exists(), "nothing written");
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
fn config_set_updates_only_the_selected_field() {
    let (root, config) = fixture();
    let before = Config::load(Some(Path::new(&config))).expect("load before");

    let updated = run(root.path(), &["config", "--set", "base=develop", "-c", &config]);

    assert_eq!(updated.status.code(), Some(0), "{}", err(&updated));
    assert_eq!(out(&updated), "");
    let after = Config::load(Some(Path::new(&config))).expect("load after");
    assert_eq!(after.base, "develop");
    assert_eq!(after.work_dir, before.work_dir);
    assert_eq!(after.repo_dirs, before.repo_dirs);
    assert_eq!(after.branch_template, before.branch_template);
}

#[test]
fn config_set_empty_clone_dir_clears_the_override() {
    let (root, config) = fixture();
    let yaml = fs::read_to_string(&config).expect("config");
    fs::write(&config, format!("{yaml}clone_dir: clones\n")).expect("clone dir");

    let updated = run(root.path(), &["config", "--set", "clone_dir=", "-c", &config]);

    assert_eq!(updated.status.code(), Some(0), "{}", err(&updated));
    let written = fs::read_to_string(&config).expect("read");
    assert!(!written.contains("clone_dir:"), "override is gone: {written}");
}

#[test]
fn config_set_unknown_field_shows_candidates() {
    let (root, config) = fixture();

    let updated = run(root.path(), &["config", "--set", "unknown=value", "-c", &config]);

    assert_eq!(updated.status.code(), Some(1));
    let message = err(&updated);
    assert!(message.contains("unknown field"), "{message}");
    assert!(message.contains("work_dir"), "{message}");
    assert!(message.contains("clone_dir"), "{message}");
}

#[test]
fn config_set_without_a_value_needs_a_terminal() {
    let (root, config) = fixture();

    let updated = run(root.path(), &["config", "--set", "base", "-c", &config]);

    assert_eq!(updated.status.code(), Some(1));
    assert!(err(&updated).contains("no terminal"), "{}", err(&updated));
}

#[test]
fn config_set_needs_an_existing_file() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("missing.yaml");

    let updated =
        run(root.path(), &["config", "--set", "base=main", "-c", path.to_str().expect("utf8")]);

    assert_eq!(updated.status.code(), Some(1));
    assert!(err(&updated).contains("missing.yaml"), "{}", err(&updated));
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
    let ticket = root.path().canonicalize().expect("root").join("tickets/PROJ-1");
    assert_eq!(out(&created), ticket.display().to_string());
    assert!(ticket.join("api/.git").exists());
}

#[test]
fn setup_runs_only_for_new_worktrees_with_repo_and_ticket_context() {
    let (root, config) = fixture();
    let existing = fs::read_to_string(&config).expect("config");
    let setup = concat!(
        "setup:\n",
        "  - 'printf \"%s/\" \"$GROVER_TICKET\" > setup.txt'\n",
        "  - 'printf \"%s\" \"$GROVER_REPO\" >> setup.txt'\n",
        "  - 'printf \"setup output\"'\n"
    );
    fs::write(&config, format!("{existing}{setup}")).expect("config with setup");

    let created = run(root.path(), &["new", "PROJ 1/foo", "-r", "api", "-c", &config]);
    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    let ticket_dir = root.path().canonicalize().expect("root").join("tickets/PROJ-1-foo");
    assert_eq!(out(&created), ticket_dir.display().to_string());
    assert!(err(&created).contains("setup output"), "{}", err(&created));
    let setup_file = root.path().join("tickets/PROJ-1-foo/api/setup.txt");
    assert_eq!(fs::read_to_string(&setup_file).expect("setup output"), "PROJ-1-foo/api");

    fs::remove_file(&setup_file).expect("remove setup output");
    let skipped = run(root.path(), &["new", "PROJ 1/foo", "-r", "api", "-c", &config]);
    assert_eq!(skipped.status.code(), Some(0), "{}", err(&skipped));
    assert!(!setup_file.exists(), "existing worktrees do not run setup again");
}

#[test]
fn setup_dry_run_logs_commands_without_running_them() {
    let (root, config) = fixture();
    let existing = fs::read_to_string(&config).expect("config");
    fs::write(&config, format!("{existing}setup:\n  - \"touch marker\"\n"))
        .expect("config with setup");

    let dry_run = run(root.path(), &["new", "PROJ-2", "-r", "api", "-n", "-c", &config]);

    assert_eq!(dry_run.status.code(), Some(0), "{}", err(&dry_run));
    assert!(err(&dry_run).contains("touch marker"), "{}", err(&dry_run));
    assert!(!root.path().join("marker").exists(), "setup command is not run");
    assert!(!root.path().join("tickets/PROJ-2").exists(), "dry-run writes nothing");
}

#[test]
fn failed_setup_warns_but_keeps_the_worktree_and_succeeds() {
    let (root, config) = fixture();
    let existing = fs::read_to_string(&config).expect("config");
    fs::write(
        &config,
        format!("{existing}setup:\n  - \"exit 23\"\n  - \"printf done > after-failure.txt\"\n"),
    )
    .expect("config with failing setup");

    let created = run(root.path(), &["new", "PROJ-3", "-r", "api", "-c", &config]);

    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    assert!(err(&created).contains("exit status: 23"), "{}", err(&created));
    let worktree = root.path().join("tickets/PROJ-3/api");
    assert!(worktree.join(".git").exists());
    assert_eq!(fs::read_to_string(worktree.join("after-failure.txt")).expect("next setup"), "done");
}

#[test]
fn new_with_a_title_puts_its_slug_in_the_branch() {
    let (root, config) = fixture();
    let created = run(
        root.path(),
        &["new", "PROJ-2", "-r", "api", "-c", &config, "--title", "Login redirect loop"],
    );

    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    let ticket = root.path().canonicalize().expect("root").join("tickets/PROJ-2");
    assert_eq!(out(&created), ticket.display().to_string(), "the path keeps the ticket slug");
    let branch = git_out(&ticket.join("api"), &["branch", "--show-current"]);
    assert_eq!(branch, "ticket/PROJ-2-login-redirect-loop");
}

#[test]
fn a_title_without_usable_characters_keeps_the_plain_branch() {
    let (root, config) = fixture();
    let created =
        run(root.path(), &["new", "PROJ-3", "-r", "api", "-c", &config, "--title", "!!!"]);

    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    assert!(err(&created).contains("yields no slug"), "{}", err(&created));
    let ticket = root.path().canonicalize().expect("root").join("tickets/PROJ-3");
    assert_eq!(git_out(&ticket.join("api"), &["branch", "--show-current"]), "ticket/PROJ-3");
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
fn rm_without_a_terminal_never_guesses() {
    let (root, config) = fixture();
    let created = run(root.path(), &["new", "PROJ-1", "-r", "api", "-c", &config]);
    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    let ticket = root.path().join("tickets/PROJ-1");

    let no_ticket = run(root.path(), &["rm", "-c", &config]);
    assert_eq!(no_ticket.status.code(), Some(1));
    assert!(err(&no_ticket).contains("missing ticket"), "{}", err(&no_ticket));

    let unknown = run(root.path(), &["rm", "PROJ-9", "-c", &config]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(err(&unknown).contains("similar"), "{}", err(&unknown));
    assert!(ticket.join("api/.git").exists(), "nothing removed");
}

#[test]
fn rm_removes_the_ticket_and_prints_nothing() {
    let (root, config) = fixture();
    let created = run(root.path(), &["new", "PROJ-1", "-r", "api", "-c", &config]);
    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    let ticket = root.path().join("tickets/PROJ-1");

    let removed = run(root.path(), &["rm", "PROJ-1", "-c", &config]);

    assert_eq!(removed.status.code(), Some(0), "{}", err(&removed));
    assert_eq!(out(&removed), "", "stdout stays empty");
    assert!(!ticket.exists());
}

#[test]
fn list_without_a_work_dir_is_empty() {
    let (root, config) = fixture();
    let listed = run(root.path(), &["list", "-c", &config]);

    assert_eq!(listed.status.code(), Some(0), "{}", err(&listed));
    assert_eq!(out(&listed), "");
    assert!(!root.path().join("tickets").exists());
}

#[test]
fn path_prints_the_ticket_dir_from_the_argument_or_the_cwd() {
    let (root, config) = fixture();
    let created = run(root.path(), &["new", "PROJ-1", "-r", "api", "-c", &config]);
    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    let ticket = root.path().canonicalize().expect("root").join("tickets/PROJ-1");

    let named = run(root.path(), &["path", "PROJ-1", "-c", &config]);
    assert_eq!(named.status.code(), Some(0), "{}", err(&named));
    assert_eq!(out(&named), ticket.display().to_string());

    let inside = run(&ticket.join("api"), &["path", "-c", &config]);
    assert_eq!(inside.status.code(), Some(0), "{}", err(&inside));
    assert_eq!(out(&inside), ticket.display().to_string());

    let outside = run(root.path(), &["path", "-c", &config]);
    assert_eq!(outside.status.code(), Some(1));
    assert_eq!(out(&outside), "", "stdout stays empty");
}

#[test]
fn status_prints_a_table_for_a_clean_and_a_dirty_repo() {
    let (root, config) = fixture();
    let web = root.path().join("code/web");
    fs::create_dir_all(&web).expect("web");
    git(&web, &["init", "-q"]);
    git(&web, &["commit", "-q", "--allow-empty", "-m", "init"]);

    let created = run(root.path(), &["new", "PROJ-1", "-r", "api", "-r", "web", "-c", &config]);
    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    let ticket = root.path().join("tickets/PROJ-1");
    fs::write(ticket.join("web/notes.txt"), "uncommitted").expect("dirty");
    fs::write(ticket.join("notes.txt"), "not a repo").expect("foreign");

    let listed = run(root.path(), &["status", "PROJ-1", "-c", &config]);
    assert_eq!(listed.status.code(), Some(0), "{}", err(&listed));
    assert_eq!(
        out(&listed),
        "repo  branch         ahead/behind  dirty
\
         api   ticket/PROJ-1  -             clean
\
         web   ticket/PROJ-1  -             1 file"
    );

    let inside = run(&ticket.join("web"), &["status", "-c", &config]);
    assert_eq!(inside.status.code(), Some(0), "{}", err(&inside));
    assert_eq!(out(&inside), out(&listed));
}

#[test]
fn status_without_a_ticket_outside_the_work_dir_fails() {
    let (root, config) = fixture();
    let outside = run(root.path(), &["status", "-c", &config]);
    assert_eq!(outside.status.code(), Some(1));
    assert_eq!(out(&outside), "", "stdout stays empty");

    let created = run(root.path(), &["new", "PROJ-1", "-r", "api", "-c", &config]);
    assert_eq!(created.status.code(), Some(0), "{}", err(&created));
    let unknown = run(root.path(), &["status", "ZZZ-9", "-c", &config]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(err(&unknown).contains("similar"), "{}", err(&unknown));
}
