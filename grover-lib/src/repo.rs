//! Find repos (`repo_dirs`, `repos`) and select them.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::fuzzy;
use crate::git;

/// Where a repo comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Child of a directory from `repo_dirs`.
    Local,
    /// From `repos`, cloned from this URL on demand.
    Remote(String),
}

/// One candidate for a worktree.
#[derive(Debug, Clone)]
pub struct Repo {
    pub name: String,
    pub path: PathBuf,
    pub source: Source,
}

/// All known repos sorted by name; local entries win on duplicate names.
pub fn discover(config: &Config) -> Vec<Repo> {
    let mut repos = local_repos(config);
    let Some(clone_dir) = &config.clone_dir else {
        // Without `clone_dir` there is nowhere to clone to; local repos still work.
        if !config.repos.is_empty() || !config.discover.is_empty() {
            tracing::warn!("`repos`/`discover` ignored: set `clone_dir` to clone them");
        }
        return repos;
    };
    for (url, declared) in remote_entries(config) {
        let Some(name) = declared.or_else(|| url_name(&url)) else {
            continue;
        };
        let path = clone_dir.join(&name);
        push(&mut repos, Repo { name, path, source: Source::Remote(url) });
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    repos
}

/// Remote repos: the `repos` list first, then whatever `discover` prints.
fn remote_entries(config: &Config) -> Vec<(String, Option<String>)> {
    let mut entries: Vec<(String, Option<String>)> =
        config.repos.iter().map(|repo| (repo.url.clone(), repo.resolved_name())).collect();
    for command in &config.discover {
        match run_discover(command) {
            Ok(urls) => entries.extend(urls.into_iter().map(|url| (url, None))),
            Err(err) => tracing::warn!("{err}"),
        }
    }
    entries
}

/// Clone URLs from one `discover` command: one per line, `#` starts a comment.
fn run_discover(command: &str) -> Result<Vec<String>> {
    // shortcut: no deadline, so a hanging CLI blocks `new`; add one when it bites.
    let out = Command::new("sh")
        .arg("-c")
        .arg(command)
        // Credentials must fail instead of prompting on a terminal grover owns.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(Error::Spawn)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(Error::Git { command: command.to_string(), stderr });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .collect())
}

fn local_repos(config: &Config) -> Vec<Repo> {
    let mut repos: Vec<Repo> = Vec::new();
    for dir in &config.repo_dirs {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::warn!("{}: {err}", dir.display());
                continue;
            }
        };
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(err) => {
                    tracing::warn!("{}: {err}", dir.display());
                    continue;
                }
            };
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
            if !git::is_repo(&path) {
                continue;
            }
            push(&mut repos, Repo { name: name.to_string(), path, source: Source::Local });
        }
    }
    repos
}

/// Selection: `all` or one fuzzy match per request, duplicates once.
pub fn select<'a>(repos: &'a [Repo], requests: &[String], all: bool) -> Result<Vec<&'a Repo>> {
    if all {
        if repos.is_empty() {
            return Err(Error::NoRepos);
        }
        return Ok(repos.iter().collect());
    }
    if requests.is_empty() {
        return Err(Error::NoSelection);
    }
    let names: Vec<&str> = repos.iter().map(|repo| repo.name.as_str()).collect();
    let mut picked: Vec<&Repo> = Vec::new();
    let queries = requests.iter().flat_map(|request| request.split(','));
    for query in queries.map(str::trim).filter(|query| !query.is_empty()) {
        let repo = &repos[fuzzy::select(&names, query)?];
        if !picked.iter().any(|other| other.name == repo.name) {
            picked.push(repo);
        }
    }
    if picked.is_empty() {
        return Err(Error::NoRepos);
    }
    Ok(picked)
}

/// `api` from `git@github.com:org/api.git` or `https://host/org/api/`.
pub fn url_name(url: &str) -> Option<String> {
    let tail = url.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = tail.strip_suffix(".git").unwrap_or(tail);
    (!name.is_empty()).then(|| name.to_string())
}

fn push(repos: &mut Vec<Repo>, repo: Repo) {
    if !repos.iter().any(|known| known.name == repo.name) {
        repos.push(repo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_from_url() {
        assert_eq!(url_name("git@github.com:org/api.git").as_deref(), Some("api"));
        assert_eq!(url_name("https://github.com/org/web/").as_deref(), Some("web"));
        assert_eq!(url_name("").as_deref(), None);
        assert_eq!(url_name(".git").as_deref(), None);
    }

    fn repos() -> Vec<Repo> {
        ["api", "web"]
            .iter()
            .map(|name| Repo {
                name: (*name).to_string(),
                path: PathBuf::from(name),
                source: Source::Local,
            })
            .collect()
    }

    #[test]
    fn all_takes_every_repo_in_order() {
        let repos = repos();
        let picked = select(&repos, &[], true).expect("all");
        let names: Vec<&str> = picked.iter().map(|repo| repo.name.as_str()).collect();
        assert_eq!(names, ["api", "web"]);
    }

    #[test]
    fn requests_keep_order_and_drop_duplicates() {
        let repos = repos();
        let picked =
            select(&repos, &["web,api".to_string(), "api".to_string()], false).expect("selection");
        let names: Vec<&str> = picked.iter().map(|repo| repo.name.as_str()).collect();
        assert_eq!(names, ["web", "api"]);
    }

    #[test]
    fn without_request_and_all_no_plan() {
        let repos = repos();
        assert!(matches!(select(&repos, &[], false), Err(Error::NoSelection)));
    }

    #[test]
    fn discover_commands_add_remote_repos() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("grover.yaml");
        let yaml = format!(
            "work_dir: tickets\nclone_dir: \"{}\"\nrepo_dirs: [code]\n\
             discover:\n  - \"printf '%s\\n' git@github.com:org/api.git \
             git@github.com:org/web.git\"\n  - \"echo git@github.com:org/api.git  # dup\"\n",
            tmp.path().join("clones").display()
        );
        fs::write(&path, yaml).expect("config");
        let config = Config::load(Some(&path)).expect("config");

        let names: Vec<String> = discover(&config).into_iter().map(|repo| repo.name).collect();
        assert_eq!(names, ["api", "web"], "duplicates collapse to one repo");
    }

    #[test]
    fn a_failing_discover_command_only_warns() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("grover.yaml");
        let yaml = format!(
            "work_dir: tickets\nclone_dir: \"{}\"\n\
             discover:\n  - \"exit 3\"\n",
            tmp.path().join("clones").display()
        );
        fs::write(&path, yaml).expect("config");
        let config = Config::load(Some(&path)).expect("config");
        assert!(discover(&config).is_empty(), "warning, no repos, no error");
    }

    #[test]
    fn without_clone_dir_remotes_are_skipped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("grover.yaml");
        fs::write(
            &path,
            "work_dir: tickets\nrepo_dirs: [code]\ndiscover:\n  - \"echo git@github.com:o/a.git\"\n",
        )
        .expect("config");
        let config = Config::load(Some(&path)).expect("config");
        assert!(discover(&config).is_empty());
    }
}
