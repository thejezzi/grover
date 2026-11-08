//! Helpers shared by the integration tests.

use std::path::Path;
use std::process::Command;

/// `git` in `dir` with a fixed identity; returns trimmed stdout, fails the test otherwise.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=grover", "-c", "user.email=grover@example.com"])
        .args(args)
        .output()
        .expect("git starts");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
