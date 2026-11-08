//! Build the plan (read only) and apply it (writes).

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Config, CopySpec};
use crate::error::{Error, Result};
use crate::git;
use crate::repo::{self, Repo, Source};

/// What the caller wants.
#[derive(Debug, Default)]
pub struct Request {
    pub ticket: String,
    pub repos: Vec<String>,
    pub all: bool,
}

/// What will be created.
#[derive(Debug)]
pub struct Plan {
    pub ticket_dir: PathBuf,
    pub branch: String,
    pub base: String,
    pub worktrees: Vec<Worktree>,
    pub copies: Vec<Copy>,
}

#[derive(Debug)]
pub struct Worktree {
    pub repo: Repo,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct Copy {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// What `apply` did or, in a dry run, would do.
#[derive(Debug, Default)]
pub struct Report {
    pub created: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub cloned: Vec<PathBuf>,
}

impl Plan {
    /// Resolve repos and compute targets; writes nothing.
    pub fn build(config: &Config, request: &Request) -> Result<Plan> {
        let slug = slug(&request.ticket)?;
        let branch = config.branch_template.replace("{ticket}", &slug);
        let ticket_dir = config.work_dir.join(&slug);

        let repos = repo::discover(config);
        let chosen = repo::select(&repos, &request.repos, request.all)?;
        let worktrees = chosen
            .into_iter()
            .map(|repo| Worktree { path: ticket_dir.join(&repo.name), repo: repo.clone() })
            .collect();

        let copies = config
            .copy
            .iter()
            .map(|spec| copy_target(&ticket_dir, spec))
            .collect::<Result<Vec<_>>>()?;
        Ok(Plan { ticket_dir, branch, base: config.base.clone(), worktrees, copies })
    }

    /// Create, skip what exists; `dry_run` writes nothing.
    pub fn apply(&self, dry_run: bool) -> Result<Report> {
        let mut report = Report::default();
        self.ensure_ticket_dir(dry_run, &mut report)?;
        for worktree in &self.worktrees {
            self.ensure_worktree(worktree, dry_run, &mut report)?;
        }
        for copy in &self.copies {
            Self::ensure_copy(copy, dry_run, &mut report)?;
        }
        Ok(report)
    }

    fn ensure_ticket_dir(&self, dry_run: bool, report: &mut Report) -> Result<()> {
        if self.ticket_dir.exists() {
            tracing::warn!("{} already exists", self.ticket_dir.display());
            report.skipped.push(self.ticket_dir.clone());
            return Ok(());
        }
        tracing::info!("creating {}", self.ticket_dir.display());
        if !dry_run {
            create_dir(&self.ticket_dir)?;
        }
        report.created.push(self.ticket_dir.clone());
        Ok(())
    }

    fn ensure_worktree(
        &self,
        worktree: &Worktree,
        dry_run: bool,
        report: &mut Report,
    ) -> Result<()> {
        if let Source::Remote(url) = &worktree.repo.source {
            if !worktree.repo.path.exists() {
                tracing::info!("cloning {url} to {}", worktree.repo.path.display());
                if !dry_run {
                    git::clone(url, &worktree.repo.path)?;
                }
                report.cloned.push(worktree.repo.path.clone());
            }
        }
        if worktree.path.exists() {
            tracing::warn!("{} already exists", worktree.path.display());
            report.skipped.push(worktree.path.clone());
            return Ok(());
        }
        let (name, path) = (&worktree.repo.name, worktree.path.display());
        tracing::info!("worktree {name} ({}) in {path}", self.branch);
        if !dry_run {
            git::worktree_add(&worktree.repo.path, &worktree.path, &self.branch, &self.base)?;
        }
        report.created.push(worktree.path.clone());
        Ok(())
    }

    fn ensure_copy(copy: &Copy, dry_run: bool, report: &mut Report) -> Result<()> {
        if copy.to.exists() {
            tracing::warn!("{} already exists", copy.to.display());
            report.skipped.push(copy.to.clone());
            return Ok(());
        }
        tracing::info!("copying {} to {}", copy.from.display(), copy.to.display());
        if !dry_run {
            if let Some(parent) = copy.to.parent() {
                create_dir(parent)?;
            }
            fs::copy(&copy.from, &copy.to)
                .map_err(|source| Error::Io { path: copy.from.clone(), source })?;
        }
        report.created.push(copy.to.clone());
        Ok(())
    }
}

/// Existing work items: direct ticket directories in `work_dir`, sorted by name.
pub fn existing_tickets(config: &Config) -> Result<Vec<String>> {
    let entries = match fs::read_dir(&config.work_dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(Error::Io { path: config.work_dir.clone(), source });
        }
    };
    let mut tickets: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    tickets.sort();
    Ok(tickets)
}

/// Ticket slug: only `[A-Za-z0-9._-]`, no leading `-`/`.`.
pub fn slug(ticket: &str) -> Result<String> {
    let mut slug = String::new();
    for char in ticket.chars() {
        if char.is_ascii_alphanumeric() || char == '.' || char == '_' {
            slug.push(char);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches(['-', '.']);
    if slug.is_empty() {
        return Err(Error::InvalidTicket(ticket.to_string()));
    }
    Ok(slug.to_string())
}

fn copy_target(ticket_dir: &Path, spec: &CopySpec) -> Result<Copy> {
    let name = spec.to.clone().or_else(|| spec.from.file_name().map(PathBuf::from));
    let Some(name) = name else {
        return Err(Error::ConfigInvalid(format!(
            "`copy`: {} has no file name",
            spec.from.display()
        )));
    };
    Ok(Copy { from: spec.from.clone(), to: ticket_dir.join(name) })
}

fn create_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_normalizes() {
        assert_eq!(slug("PROJ 123/foo").expect("slug"), "PROJ-123-foo");
        assert_eq!(slug("PROJ-123").expect("slug"), "PROJ-123");
        assert_eq!(slug("  proj 42 ").expect("slug"), "proj-42");
        assert_eq!(slug("tickets/PROJ-9/").expect("slug"), "tickets-PROJ-9");
    }

    #[test]
    fn empty_slug_is_an_error() {
        assert!(matches!(slug(" -/. "), Err(Error::InvalidTicket(_))));
    }
}
