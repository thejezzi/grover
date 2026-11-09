//! Plan a ticket directory (`new`), then list, resolve, inspect and remove tickets.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::config::{Config, CopySpec};
use crate::error::{Error, Result};
use crate::fuzzy;
use crate::git;
use crate::repo::{self, Repo, Source};

/// What the caller wants.
#[derive(Debug, Default)]
pub struct Request {
    pub ticket: String,
    pub repos: Vec<String>,
    pub all: bool,
    /// Optional ticket title; appended to the ticket slug in the branch name.
    pub title: Option<String>,
}

/// What will be created.
#[derive(Debug)]
pub struct Plan {
    pub ticket_dir: PathBuf,
    pub ticket: String,
    pub branch: String,
    /// `false` leaves the worktrees detached at the base instead of creating `branch`.
    pub create_branch: bool,
    pub base: String,
    pub setup: Vec<String>,
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
        let branch = branch_name(config, &slug, request);
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
        Ok(Plan {
            ticket_dir,
            ticket: slug,
            branch,
            create_branch: config.create_branch,
            base: config.base.clone(),
            setup: config.setup.clone(),
            worktrees,
            copies,
        })
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
        let path = worktree.path.display();
        let branch = if self.create_branch { self.branch.as_str() } else { "detached" };
        tracing::info!("worktree {} ({branch}) in {path}", worktree.repo.name);
        if dry_run {
            for command in &self.setup {
                tracing::info!("would run setup in {path}: {command}");
            }
        } else {
            let branch = self.create_branch.then_some(self.branch.as_str());
            git::worktree_add(&worktree.repo.path, &worktree.path, branch, &self.base)?;
            self.run_setup(worktree);
        }
        report.created.push(worktree.path.clone());
        Ok(())
    }

    fn run_setup(&self, worktree: &Worktree) {
        for (index, command) in self.setup.iter().enumerate() {
            tracing::info!(
                "running setup command {}/{} for repo {}",
                index + 1,
                self.setup.len(),
                worktree.repo.name
            );
            // Keep the ticket path as the only stdout line from `new`.
            let script = format!("exec 1>&2\n{command}");
            let result = Command::new("sh")
                .arg("-c")
                .arg(script)
                .current_dir(&worktree.path)
                .env("GROVER_REPO", &worktree.repo.name)
                .env("GROVER_TICKET", &self.ticket)
                .status();
            match result {
                Ok(status) if status.success() => {}
                Ok(status) => tracing::warn!(
                    "setup command {}/{} for repo {} failed ({status})",
                    index + 1,
                    self.setup.len(),
                    worktree.repo.name
                ),
                Err(err) => tracing::warn!(
                    "could not start setup command {}/{} for repo {}: {err}",
                    index + 1,
                    self.setup.len(),
                    worktree.repo.name
                ),
            }
        }
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
    let mut tickets: Vec<String> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!("{}: {err}", config.work_dir.display());
                continue;
            }
        };
        if entry.path().is_dir() {
            tickets.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    tickets.sort();
    Ok(tickets)
}

/// Ticket directory for `ticket`: exact, never guessed, because `rm` deletes.
pub fn resolve_ticket(config: &Config, ticket: &str) -> Result<PathBuf> {
    let slug = slug(ticket)?;
    let tickets = existing_tickets(config)?;
    let known: Vec<&str> = tickets.iter().map(String::as_str).collect();
    if !known.contains(&slug.as_str()) {
        let suggestions = fuzzy::similar(&known, &slug);
        return Err(Error::UnknownName { query: ticket.to_string(), suggestions });
    }
    Ok(config.work_dir.join(slug))
}

/// Ticket directory for `ticket` to read from: exact, else the single fuzzy match.
pub fn find_ticket(config: &Config, ticket: &str) -> Result<PathBuf> {
    let slug = slug(ticket)?;
    let tickets = existing_tickets(config)?;
    let known: Vec<&str> = tickets.iter().map(String::as_str).collect();
    let index = match known.iter().position(|name| *name == slug.as_str()) {
        Some(index) => index,
        None => fuzzy::select(&known, &slug)?,
    };
    Ok(config.work_dir.join(&tickets[index]))
}

/// Ticket directory above `dir`: the first path segment below `work_dir`.
pub fn ticket_above(config: &Config, dir: &Path) -> Result<PathBuf> {
    // shortcut: paths are compared as written, so a symlinked work_dir never matches.
    let work_dir = &config.work_dir;
    let segment = dir.strip_prefix(work_dir).ok().and_then(|rest| rest.components().next());
    match segment {
        Some(Component::Normal(name)) => Ok(work_dir.join(name)),
        _ => Err(Error::NotInWorkDir { dir: dir.to_path_buf(), work_dir: work_dir.clone() }),
    }
}

/// Remove a ticket: worktrees and branches, the `copy` files, then the directory.
/// A failed step is a warning, so one dirty worktree does not stop the rest.
pub fn remove(config: &Config, ticket_dir: &Path, force: bool, dry_run: bool) -> Result<()> {
    // Collected before any removal: deleting while `read_dir` walks can skip an entry.
    let mut worktrees: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(ticket_dir)
        .map_err(|source| Error::Io { path: ticket_dir.to_path_buf(), source })?
    {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(err) => {
                tracing::warn!("{}: {err}", ticket_dir.display());
                continue;
            }
        };
        // `.git` as a file is a worktree; every other child is foreign and stays.
        if path.join(".git").is_file() {
            worktrees.push(path);
        }
    }
    for worktree in &worktrees {
        if let Err(err) = remove_worktree(worktree, force, dry_run) {
            tracing::warn!("{err}");
        }
    }
    for spec in &config.copy {
        match copy_target(ticket_dir, spec) {
            Ok(copy) => remove_file(&copy.to, dry_run),
            Err(err) => tracing::warn!("{err}"),
        }
    }
    tracing::info!("removing {}", ticket_dir.display());
    if dry_run {
        return Ok(());
    }
    // `remove_dir`, not `remove_dir_all`: what grover did not create stays.
    if let Err(err) = fs::remove_dir(ticket_dir) {
        tracing::warn!("{} kept: {err}", ticket_dir.display());
    }
    Ok(())
}

/// One worktree of a ticket and what git says about it.
#[derive(Debug)]
pub struct RepoStatus {
    pub name: String,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    /// Commits ahead of the upstream; `None` without one.
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
    pub dirty: usize,
}

/// State of every worktree in the ticket, sorted by repository name.
pub fn statuses(ticket_dir: &Path) -> Result<Vec<RepoStatus>> {
    let mut statuses = Vec::new();
    for entry in fs::read_dir(ticket_dir)
        .map_err(|source| Error::Io { path: ticket_dir.to_path_buf(), source })?
    {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(err) => {
                tracing::warn!("{}: {err}", ticket_dir.display());
                continue;
            }
        };
        // A `.git` file is a worktree; foreign entries are ignored.
        if !path.join(".git").is_file() {
            continue;
        }
        let (ahead, behind) = match git::ahead_behind(&path)? {
            Some((ahead, behind)) => (Some(ahead), Some(behind)),
            None => (None, None),
        };
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
        statuses.push(RepoStatus {
            name: name.unwrap_or_default(),
            branch: git::head_branch(&path)?,
            ahead,
            behind,
            dirty: git::changed_files(&path)?,
        });
    }
    statuses.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(statuses)
}

/// One worktree and its branch; branch and repo come from the worktree itself.
fn remove_worktree(worktree: &Path, force: bool, dry_run: bool) -> Result<()> {
    let repo = git::common_dir(worktree)?;
    let branch = git::head_branch(worktree)?;
    tracing::info!("removing worktree {}", worktree.display());
    if let Some(branch) = &branch {
        tracing::info!("deleting branch {branch}");
    }
    if dry_run {
        return Ok(());
    }
    git::worktree_remove(&repo, worktree, force)?;
    git::worktree_prune(&repo)?;
    match branch {
        Some(branch) => git::branch_delete(&repo, &branch, force),
        None => Ok(()),
    }
}

/// Branch name: the template with the ticket slug, plus the title slug when given.
fn branch_name(config: &Config, ticket_slug: &str, request: &Request) -> String {
    let title = request.title.as_deref().and_then(|title| {
        title_slug(title).or_else(|| {
            tracing::warn!("title {title:?} yields no slug; using the ticket only");
            None
        })
    });
    let Some(title) = title else {
        return config.branch_template.replace("{ticket}", ticket_slug);
    };
    if !config.create_branch {
        tracing::warn!("title slug ignored, `create_branch` is off");
    }
    config.branch_template.replace("{ticket}", &format!("{ticket_slug}-{title}"))
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

/// Words that carry no meaning in a branch name; German ones are included because
/// the titles come from a German ticket system.
const FILLER: [&str; 19] = [
    "a", "an", "the", "of", "to", "for", "and", "in", "on", "with", "der", "die", "das", "und",
    "von", "fuer", "im", "zu", "mit",
];

/// Max words and characters a title slug keeps.
const TITLE_WORDS: usize = 5;
const TITLE_CHARS: usize = 48;

/// Branch-safe slug from a ticket title: lowercase, filler words dropped,
/// capped at five words and 48 characters. `None` when nothing usable is left.
fn title_slug(title: &str) -> Option<String> {
    let cleaned = transliterate(title);
    let words: Vec<&str> = cleaned.split('-').filter(|word| !word.is_empty()).collect();
    let meaningful: Vec<&str> =
        words.iter().copied().filter(|word| !FILLER.contains(word)).collect();
    // A title of nothing but filler words is still a title; keep them rather than fail.
    let words = if meaningful.is_empty() { words } else { meaningful };
    let mut slug = String::new();
    for word in words.into_iter().take(TITLE_WORDS) {
        let next = if slug.is_empty() { word.len() } else { slug.len() + 1 + word.len() };
        if next > TITLE_CHARS {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(word);
    }
    (!slug.is_empty()).then_some(slug)
}

/// Lowercase ASCII words joined by single dashes: `Über Größe` becomes `ueber-groesse`.
fn transliterate(text: &str) -> String {
    let mut out = String::new();
    for char in text.chars() {
        match char {
            'ä' | 'Ä' => out.push_str("ae"),
            'ö' | 'Ö' => out.push_str("oe"),
            'ü' | 'Ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            char if char.is_ascii_alphanumeric() => out.push(char.to_ascii_lowercase()),
            _ if !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    out.trim_matches('-').to_string()
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

fn remove_file(path: &Path, dry_run: bool) {
    if !path.exists() {
        return;
    }
    tracing::info!("removing {}", path.display());
    if dry_run {
        return;
    }
    if let Err(err) = fs::remove_file(path) {
        tracing::warn!("{} kept: {err}", path.display());
    }
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
    fn a_ticket_is_found_exactly_or_by_its_only_fuzzy_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        for ticket in ["PROJ-1", "PROJ-12", "ABC-7"] {
            fs::create_dir(dir.path().join(ticket)).expect("ticket");
        }
        let config = Config::new(dir.path().to_path_buf());

        assert_eq!(find_ticket(&config, "PROJ-12").expect("exact"), dir.path().join("PROJ-12"));
        assert_eq!(find_ticket(&config, "proj-3").expect("fuzzy"), dir.path().join("PROJ-1"));
        assert!(matches!(find_ticket(&config, "PROJ"), Err(Error::AmbiguousName { .. })));
        assert!(matches!(find_ticket(&config, "ZZZ"), Err(Error::UnknownName { .. })));
    }

    #[test]
    fn the_ticket_above_is_the_first_segment_below_the_work_dir() {
        let config = Config::new(PathBuf::from("/work/tickets"));

        let inside = ticket_above(&config, Path::new("/work/tickets/PROJ-1/api/src"));
        assert_eq!(inside.expect("above"), PathBuf::from("/work/tickets/PROJ-1"));

        let at_work_dir = ticket_above(&config, Path::new("/work/tickets")).expect_err("work dir");
        assert!(at_work_dir.to_string().contains("no ticket folder above"), "{at_work_dir}");
        assert!(ticket_above(&config, Path::new("/elsewhere/PROJ-1")).is_err());
    }

    #[test]
    fn empty_slug_is_an_error() {
        assert!(matches!(slug(" -/. "), Err(Error::InvalidTicket(_))));
    }

    #[test]
    fn a_title_becomes_a_branch_slug() {
        assert_eq!(title_slug("Login redirect loop").expect("slug"), "login-redirect-loop");
        assert_eq!(title_slug("Fix the bug in the login").expect("slug"), "fix-bug-login");
        assert_eq!(title_slug("  Mehr   Platz  ").expect("slug"), "mehr-platz");
        assert_eq!(title_slug("A/B-Test: 50% roll-out").expect("slug"), "b-test-50-roll-out");
    }

    #[test]
    fn a_title_slug_transliterates_umlauts() {
        assert_eq!(title_slug("Über Größe").expect("slug"), "ueber-groesse");
        assert_eq!(title_slug("Prüfung für Straße").expect("slug"), "pruefung-strasse");
    }

    #[test]
    fn a_title_slug_keeps_five_words_and_forty_eight_characters() {
        let slug = title_slug("one two three four five six seven").expect("slug");
        assert_eq!(slug, "one-two-three-four-five");
        let long =
            title_slug("aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd eeeeeeeeee").expect("slug");
        assert!(long.len() <= TITLE_CHARS, "{long} is {} chars", long.len());
        assert!(long.starts_with("aaaaaaaaaa-bbbbbbbbbb"), "{long}");
    }

    #[test]
    fn a_title_of_only_filler_words_keeps_them() {
        assert_eq!(title_slug("the of and").expect("slug"), "the-of-and");
    }

    #[test]
    fn a_title_without_letters_or_digits_has_no_slug() {
        assert_eq!(title_slug("---"), None);
        assert_eq!(title_slug("   "), None);
    }

    #[test]
    fn a_title_extends_the_branch_name() {
        let config = Config::new(PathBuf::from("/work/tickets"));
        let plain = Request { ticket: "PROJ-1".into(), ..Request::default() };
        assert_eq!(branch_name(&config, "PROJ-1", &plain), "ticket/PROJ-1");

        let titled = Request {
            ticket: "PROJ-1".into(),
            title: Some("Login redirect loop".into()),
            ..Request::default()
        };
        assert_eq!(branch_name(&config, "PROJ-1", &titled), "ticket/PROJ-1-login-redirect-loop");
    }

    #[test]
    fn the_title_slug_follows_the_template() {
        let mut config = Config::new(PathBuf::from("/work/tickets"));
        config.branch_template = "feat/{ticket}".into();
        let request = Request {
            ticket: "PROJ-9".into(),
            title: Some("Add dark mode".into()),
            ..Request::default()
        };
        assert_eq!(branch_name(&config, "PROJ-9", &request), "feat/PROJ-9-add-dark-mode");
    }

    #[test]
    fn a_title_without_usable_words_falls_back_to_the_ticket() {
        let config = Config::new(PathBuf::from("/work/tickets"));
        let request =
            Request { ticket: "PROJ-2".into(), title: Some("---".into()), ..Request::default() };
        assert_eq!(branch_name(&config, "PROJ-2", &request), "ticket/PROJ-2");
    }
}
