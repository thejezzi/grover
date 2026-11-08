//! Find repos (`repo_dirs`, `repos`) and select them.

use std::fs;
use std::path::PathBuf;

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
    for remote in &config.repos {
        let (Some(name), Some(clone_dir)) = (remote.resolved_name(), &config.clone_dir) else {
            continue;
        };
        let path = clone_dir.join(&name);
        push(&mut repos, Repo { name, path, source: Source::Remote(remote.url.clone()) });
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
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
}
