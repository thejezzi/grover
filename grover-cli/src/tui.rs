//! Interactive prompts via `inquire`; the only place with TUI code.

use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use grover_lib::config::{Config, CopySpec, RemoteRepo};
use grover_lib::error::Error;
use grover_lib::repo::{self, Source};
use inquire::ui::{Attributes, Color, ErrorMessageRenderConfig, RenderConfig, StyleSheet, Styled};
use inquire::{InquireError, MultiSelect, Select, Text};

const PAGE_SIZE: usize = 12;
const INPUT_HELP: &str = "enter confirm · esc cancel";
const SELECT_HELP: &str = "↑↓ move · type filter · enter select · esc cancel";
const MULTI_SELECT_HELP: &str =
    "↑↓ move · space pick · →/← all/none · type filter · enter done · esc cancel";

/// Prompts only run on a terminal, so scripts and agents never hang.
pub fn available() -> bool {
    io::stdin().is_terminal()
}

/// What the flows ask; `None` means the user aborted.
pub trait Prompter {
    fn input(&mut self, prompt: &str, initial: &str) -> anyhow::Result<Option<String>>;
    fn select(&mut self, prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>>;
    fn multi_select(
        &mut self,
        prompt: &str,
        items: &[String],
    ) -> anyhow::Result<Option<Vec<usize>>>;
}

/// `inquire` on a real terminal: one theme, typing filters lists, stderr only.
pub struct Tui;

impl Prompter for Tui {
    fn input(&mut self, prompt: &str, initial: &str) -> anyhow::Result<Option<String>> {
        let answer = Text::new(prompt)
            .with_initial_value(initial)
            .with_help_message(INPUT_HELP)
            .with_render_config(theme())
            .prompt_skippable();
        cancelled(answer)
    }

    fn select(&mut self, prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>> {
        let picked = Select::new(prompt, items.to_vec())
            .with_page_size(PAGE_SIZE)
            .with_help_message(SELECT_HELP)
            .with_render_config(theme())
            .raw_prompt_skippable();
        Ok(cancelled(picked)?.map(|option| option.index))
    }

    fn multi_select(
        &mut self,
        prompt: &str,
        items: &[String],
    ) -> anyhow::Result<Option<Vec<usize>>> {
        let picked = MultiSelect::new(prompt, items.to_vec())
            .with_page_size(PAGE_SIZE)
            .with_help_message(MULTI_SELECT_HELP)
            .with_render_config(theme())
            .raw_prompt_skippable();
        let picked = cancelled(picked)?;
        Ok(picked.map(|options| options.into_iter().map(|option| option.index).collect()))
    }
}

/// Shared quiet theme; `NO_COLOR` keeps the layout but drops every style.
fn theme() -> RenderConfig<'static> {
    themed(std::env::var_os("NO_COLOR").is_some())
}

fn themed(no_color: bool) -> RenderConfig<'static> {
    let mut theme = RenderConfig::empty()
        .with_prompt_prefix(Styled::new("?"))
        .with_answered_prompt_prefix(Styled::new("✓"))
        .with_highlighted_option_prefix(Styled::new("❯"))
        .with_selected_checkbox(Styled::new("[✓]"))
        .with_scroll_up_prefix(Styled::new("↑"))
        .with_scroll_down_prefix(Styled::new("↓"));
    if no_color {
        return theme;
    }
    theme.prompt_prefix = theme.prompt_prefix.with_fg(Color::LightCyan).with_attr(Attributes::BOLD);
    theme.answered_prompt_prefix = theme.answered_prompt_prefix.with_fg(Color::LightGreen);
    theme.highlighted_option_prefix =
        theme.highlighted_option_prefix.with_fg(Color::LightCyan).with_attr(Attributes::BOLD);
    theme.selected_checkbox = theme.selected_checkbox.with_fg(Color::LightGreen);
    theme.unselected_checkbox = theme.unselected_checkbox.with_fg(Color::DarkGrey);
    theme.scroll_up_prefix = theme.scroll_up_prefix.with_fg(Color::DarkGrey);
    theme.scroll_down_prefix = theme.scroll_down_prefix.with_fg(Color::DarkGrey);
    theme.canceled_prompt_indicator = theme.canceled_prompt_indicator.with_fg(Color::LightRed);
    theme.prompt = StyleSheet::new().with_attr(Attributes::BOLD);
    theme.help_message = StyleSheet::new().with_fg(Color::DarkGrey);
    theme.placeholder = StyleSheet::new().with_fg(Color::DarkGrey);
    theme.answer = StyleSheet::new().with_fg(Color::LightCyan);
    theme.selected_option =
        Some(StyleSheet::new().with_fg(Color::LightCyan).with_attr(Attributes::BOLD));
    theme.error_message = ErrorMessageRenderConfig::default_colored();
    theme
}

/// ESC and Ctrl-C are an abort, not an error.
fn cancelled<T>(result: Result<Option<T>, InquireError>) -> anyhow::Result<Option<T>> {
    match result {
        Ok(answer) => Ok(answer),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Fills `new` inputs: prompts only for what is missing.
pub fn complete(
    prompter: &mut dyn Prompter,
    config: &Config,
    ticket: Option<String>,
    repos: Vec<String>,
    all: bool,
) -> anyhow::Result<(String, Vec<String>)> {
    let ticket = match ticket {
        Some(ticket) => ticket,
        None => ask_ticket(prompter)?,
    };
    if all || !repos.is_empty() {
        return Ok((ticket, repos));
    }
    Ok((ticket, ask_repos(prompter, config)?))
}

/// Ticket for `new`; the slug check happens later in `Plan::build`.
fn ask_ticket(prompter: &mut dyn Prompter) -> anyhow::Result<String> {
    prompter.input("Ticket", "")?.ok_or_else(abort)
}

/// Ticket for `rm`: pick from the existing ones, never a guess.
pub fn pick_ticket(prompter: &mut dyn Prompter, tickets: &[String]) -> anyhow::Result<String> {
    let Some(index) = prompter.select("Ticket to remove", tickets)? else {
        return Err(abort());
    };
    tickets.get(index).cloned().ok_or_else(abort)
}

/// Repos for `new`: multi-select over the discovered repos, re-asked while empty.
fn ask_repos(prompter: &mut dyn Prompter, config: &Config) -> anyhow::Result<Vec<String>> {
    let repos = repo::discover(config);
    if repos.is_empty() {
        return Err(Error::NoRepos.into());
    }
    let width = repos.iter().map(|repo| repo.name.chars().count()).max().unwrap_or(0);
    let items: Vec<String> = repos.iter().map(|repo| repo_label(repo, width)).collect();
    loop {
        let Some(picked) = prompter.multi_select("Repos", &items)? else {
            return Err(abort());
        };
        if picked.is_empty() {
            tracing::warn!("no repo picked");
            continue;
        }
        let names = picked
            .into_iter()
            .filter_map(|index| repos.get(index))
            .map(|repo| repo.name.clone())
            .collect();
        return Ok(names);
    }
}

fn repo_label(repo: &repo::Repo, width: usize) -> String {
    let source = match &repo.source {
        Source::Local => "local",
        Source::Remote(_) => "remote · clone on demand",
    };
    format!("{:<width$}  {}", repo.name, source)
}

/// Config editor: one menu over all top-level fields, written after each entry.
pub fn edit_config(
    config: &mut Config,
    path: &Path,
    prompter: &mut dyn Prompter,
) -> anyhow::Result<()> {
    loop {
        let items = menu(config);
        let title = format!("Config · {}", path.display());
        let Some(choice) = prompter.select(&title, &items)? else {
            return Err(abort());
        };
        match choice {
            0 => {
                let current = path_text(&config.work_dir);
                let answer = text(prompter, "work_dir", &current)?;
                config.work_dir = PathBuf::from(answer);
            }
            1 => {
                let current = config.branch_template.clone();
                config.branch_template = text(prompter, "branch_template", &current)?;
            }
            2 => {
                let current = config.base.clone();
                config.base = text(prompter, "base", &current)?;
            }
            3 => edit_list(
                prompter,
                "repo_dirs",
                &mut config.repo_dirs,
                |dir| path_text(dir),
                ask_dir,
            )?,
            4 => {
                let current = config.clone_dir.as_deref().map(path_text).unwrap_or_default();
                let answer = text(prompter, "clone_dir", &current)?;
                config.clone_dir = (!answer.is_empty()).then(|| PathBuf::from(answer));
            }
            5 => edit_list(prompter, "repos", &mut config.repos, remote_label, ask_remote)?,
            6 => edit_list(prompter, "copy", &mut config.copy, copy_label, ask_copy)?,
            _ => return Ok(()),
        }
        if let Err(err) = save_and_reload(config, path) {
            tracing::warn!("not saved: {err}");
        }
    }
}

/// `config --init`: ask every field once, then write. Nothing is written on error.
pub fn new_config(path: &Path, prompter: &mut dyn Prompter) -> anyhow::Result<()> {
    let work_dir = PathBuf::from(required(prompter, "work_dir")?);
    let mut config = Config::new(work_dir);
    config.branch_template = text(prompter, "branch_template", &config.branch_template)?;
    config.base = text(prompter, "base", &config.base)?;
    edit_list(prompter, "repo_dirs", &mut config.repo_dirs, |dir| path_text(dir), ask_dir)?;
    let current = config.clone_dir.as_deref().map(path_text).unwrap_or_default();
    let answer = text(prompter, "clone_dir", &current)?;
    config.clone_dir = (!answer.is_empty()).then(|| PathBuf::from(answer));
    edit_list(prompter, "repos", &mut config.repos, remote_label, ask_remote)?;
    edit_list(prompter, "copy", &mut config.copy, copy_label, ask_copy)?;
    config.validate()?;
    config.save(path)?;
    Ok(())
}

/// A value without a default: keep asking until something is typed.
fn required(prompter: &mut dyn Prompter, prompt: &str) -> anyhow::Result<String> {
    loop {
        let answer = prompted(prompter, prompt, "")?;
        if !answer.is_empty() {
            return Ok(answer);
        }
        tracing::warn!("{prompt} is required");
    }
}

/// Labels of the editor menu, in match order; the last entry leaves the editor.
fn menu(config: &Config) -> Vec<String> {
    vec![
        format!("{:<18}{}", "work_dir", path_text(&config.work_dir)),
        format!("{:<18}{}", "branch_template", config.branch_template),
        format!("{:<18}{}", "base", config.base),
        format!("{:<18}{}", "repo_dirs", count(config.repo_dirs.len())),
        format!(
            "{:<18}{}",
            "clone_dir",
            config.clone_dir.as_deref().map(path_text).unwrap_or_default()
        ),
        format!("{:<18}{}", "repos", count(config.repos.len())),
        format!("{:<18}{}", "copy", count(config.copy.len())),
        "← exit".to_string(),
    ]
}

fn count(entries: usize) -> String {
    match entries {
        1 => "1 entry".to_string(),
        entries => format!("{entries} entries"),
    }
}

/// Menu for one list field: add, remove, back.
fn edit_list<T>(
    prompter: &mut dyn Prompter,
    title: &str,
    items: &mut Vec<T>,
    label: impl Fn(&T) -> String,
    ask: impl Fn(&mut dyn Prompter) -> anyhow::Result<Option<T>>,
) -> anyhow::Result<()> {
    loop {
        let mut entries = vec![format!("+ add {title}")];
        entries.extend(items.iter().map(&label));
        entries.push("← back".to_string());
        let Some(choice) = prompter.select(title, &entries)? else {
            return Err(abort());
        };
        if choice == 0 {
            if let Some(entry) = ask(prompter)? {
                items.push(entry);
            }
        } else if choice == entries.len() - 1 {
            return Ok(());
        } else {
            items.remove(choice - 1);
        }
    }
}

fn ask_dir(prompter: &mut dyn Prompter) -> anyhow::Result<Option<PathBuf>> {
    let answer = prompted(prompter, "repo dir", "")?;
    Ok((!answer.is_empty()).then(|| PathBuf::from(answer)))
}

fn ask_remote(prompter: &mut dyn Prompter) -> anyhow::Result<Option<RemoteRepo>> {
    let url = prompted(prompter, "repo url", "")?;
    if url.is_empty() {
        return Ok(None);
    }
    let name = prompted(prompter, "name (empty = from url)", "")?;
    Ok(Some(RemoteRepo { url, name: (!name.is_empty()).then_some(name) }))
}

fn ask_copy(prompter: &mut dyn Prompter) -> anyhow::Result<Option<CopySpec>> {
    let from = prompted(prompter, "from", "")?;
    if from.is_empty() {
        return Ok(None);
    }
    let to = prompted(prompter, "to (empty = file name)", "")?;
    let to = (!to.is_empty()).then(|| PathBuf::from(to));
    Ok(Some(CopySpec { from: PathBuf::from(from), to }))
}

fn remote_label(repo: &RemoteRepo) -> String {
    match repo.resolved_name() {
        Some(name) => format!("{name}  {}", repo.url),
        None => format!("{}  (no name)", repo.url),
    }
}

fn copy_label(copy: &CopySpec) -> String {
    let from = path_text(&copy.from);
    match &copy.to {
        Some(to) => format!("{from} → {}", path_text(to)),
        None => from,
    }
}

/// One text answer, trimmed; empty keeps the current value.
fn text(prompter: &mut dyn Prompter, prompt: &str, current: &str) -> anyhow::Result<String> {
    let answer = prompted(prompter, prompt, current)?;
    Ok(if answer.is_empty() { current.to_string() } else { answer })
}

fn prompted(prompter: &mut dyn Prompter, prompt: &str, initial: &str) -> anyhow::Result<String> {
    let answer = prompter.input(prompt, initial)?.ok_or_else(abort)?;
    Ok(answer.trim().to_string())
}

fn path_text(path: &Path) -> String {
    path.display().to_string()
}

/// Validate, write, reload; on any error the file and `config` keep the old state.
fn save_and_reload(config: &mut Config, path: &Path) -> anyhow::Result<()> {
    let previous = fs::read_to_string(path)?;
    let saved = config.validate().and_then(|()| config.save(path));
    match saved.and_then(|()| Config::load(Some(path))) {
        Ok(reloaded) => {
            *config = reloaded;
            Ok(())
        }
        Err(err) => {
            fs::write(path, previous)?;
            *config = Config::load(Some(path))?;
            Err(err.into())
        }
    }
}

fn abort() -> anyhow::Error {
    anyhow::anyhow!("aborted, nothing written")
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    enum Answer {
        Input(String),
        Select(usize),
        Multi(Vec<usize>),
        Abort,
    }

    /// Scripted prompter: feeds answers and records every prompt.
    #[derive(Default)]
    struct Script {
        answers: VecDeque<Answer>,
        prompts: Vec<String>,
    }

    impl Script {
        fn new(answers: Vec<Answer>) -> Script {
            Script { answers: answers.into(), prompts: Vec::new() }
        }

        fn next(&mut self, prompt: &str) -> Answer {
            self.prompts.push(prompt.to_string());
            self.answers.pop_front().expect("scripted answer")
        }
    }

    impl Prompter for Script {
        fn input(&mut self, prompt: &str, _initial: &str) -> anyhow::Result<Option<String>> {
            match self.next(prompt) {
                Answer::Input(value) => Ok(Some(value)),
                Answer::Abort => Ok(None),
                _ => panic!("scripted answer is not an input"),
            }
        }

        fn select(&mut self, prompt: &str, _items: &[String]) -> anyhow::Result<Option<usize>> {
            match self.next(prompt) {
                Answer::Select(index) => Ok(Some(index)),
                Answer::Abort => Ok(None),
                _ => panic!("scripted answer is not a select"),
            }
        }

        fn multi_select(
            &mut self,
            prompt: &str,
            _items: &[String],
        ) -> anyhow::Result<Option<Vec<usize>>> {
            match self.next(prompt) {
                Answer::Multi(indices) => Ok(Some(indices)),
                Answer::Abort => Ok(None),
                _ => panic!("scripted answer is not a multi select"),
            }
        }
    }

    /// Config with one local and two remote repos; discovery needs no git here.
    fn config_file(dir: &Path) -> (PathBuf, Config) {
        let path = dir.join("grover.yaml");
        fs::write(&path, "work_dir: tickets\nrepo_dirs:\n  - code\n").expect("config");
        let config = Config::load(Some(&path)).expect("load");
        (path, config)
    }

    fn remote_config(path: &Path) -> Config {
        let yaml = concat!(
            "work_dir: tickets\n",
            "repos:\n",
            "  - url: git@github.com:org/api.git\n",
            "  - url: git@github.com:org/web.git\n"
        );
        fs::write(path, yaml).expect("config");
        Config::load(Some(path)).expect("load")
    }

    #[test]
    fn cancel_and_interrupt_are_aborts() {
        let cancelled_answer: Result<Option<u8>, InquireError> =
            Err(InquireError::OperationCanceled);
        assert_eq!(cancelled(cancelled_answer).expect("cancel"), None);
        let interrupted: Result<Option<u8>, InquireError> = Err(InquireError::OperationInterrupted);
        assert_eq!(cancelled(interrupted).expect("interrupt"), None);
    }

    #[test]
    fn no_color_keeps_the_layout_but_drops_styles() {
        let plain = themed(true);
        let colored = themed(false);
        assert_eq!(plain.prompt_prefix.content, colored.prompt_prefix.content);
        assert_eq!(plain.highlighted_option_prefix.content, "❯");
        assert_eq!(plain.selected_checkbox.content, "[✓]");
        assert!(plain.prompt.is_empty());
        assert!(plain.help_message.is_empty());
        assert!(plain.selected_option.is_none());
        assert_eq!(colored.help_message.fg, Some(Color::DarkGrey));
        assert!(colored.prompt.att.contains(Attributes::BOLD));
    }

    #[test]
    fn repo_labels_align_the_source_column() {
        let local =
            repo::Repo { name: "api".into(), path: PathBuf::from("api"), source: Source::Local };
        let remote = repo::Repo {
            name: "web".into(),
            path: PathBuf::from("web"),
            source: Source::Remote("url".into()),
        };
        let local = repo_label(&local, 4);
        let remote = repo_label(&remote, 4);
        assert_eq!(local.find("local"), remote.find("remote"));
    }

    #[test]
    fn ticket_is_asked_when_missing() {
        let mut script = Script::new(vec![Answer::Input("PROJ-7".into())]);
        assert_eq!(ask_ticket(&mut script).expect("ticket"), "PROJ-7");
    }

    #[test]
    fn abort_is_an_error() {
        let mut script = Script::new(vec![Answer::Abort]);
        let err = ask_ticket(&mut script).expect_err("abort");
        assert!(err.to_string().contains("aborted"), "{err}");
    }

    #[test]
    fn ticket_for_rm_is_picked_from_the_list() {
        let tickets = ["PROJ-1".to_string(), "PROJ-2".to_string()];
        let mut script = Script::new(vec![Answer::Select(1)]);
        assert_eq!(pick_ticket(&mut script, &tickets).expect("ticket"), "PROJ-2");

        let mut abort = Script::new(vec![Answer::Abort]);
        assert!(pick_ticket(&mut abort, &tickets).is_err());
    }

    #[test]
    fn empty_repo_pick_is_asked_again() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("remotes.yaml");
        let mut script = Script::new(vec![Answer::Multi(vec![]), Answer::Multi(vec![1])]);
        let repos = ask_repos(&mut script, &remote_config(&path)).expect("repos");
        assert_eq!(repos, ["web"]);
        assert_eq!(script.prompts.len(), 2);
    }

    #[test]
    fn arguments_skip_their_prompts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("remotes.yaml");
        let config = remote_config(&path);
        let mut script = Script::new(vec![]);
        let given = (Some("PROJ-1".to_string()), vec!["api".to_string()]);
        let (ticket, repos) =
            complete(&mut script, &config, given.0, given.1, false).expect("complete");
        assert_eq!(ticket, "PROJ-1");
        assert_eq!(repos, ["api"]);
        assert!(script.prompts.is_empty());
    }

    #[test]
    fn all_skips_the_repo_prompt() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("remotes.yaml");
        let mut script = Script::new(vec![Answer::Input("PROJ-2".into())]);
        let (ticket, repos) =
            complete(&mut script, &remote_config(&path), None, vec![], true).expect("complete");
        assert_eq!(ticket, "PROJ-2");
        assert!(repos.is_empty());
        assert_eq!(script.prompts, ["Ticket"]);
    }

    #[test]
    fn editing_a_value_writes_and_reloads() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let elsewhere = tmp.path().join("elsewhere");
        let mut script = Script::new(vec![
            Answer::Select(0),
            Answer::Input(elsewhere.to_str().expect("utf8").into()),
            Answer::Select(7),
        ]);
        edit_config(&mut config, &path, &mut script).expect("edit");
        assert_eq!(config.work_dir, elsewhere);
        let reloaded = Config::load(Some(&path)).expect("reload");
        assert_eq!(reloaded.work_dir, elsewhere);
    }

    #[test]
    fn invalid_value_keeps_the_old_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let before = fs::read_to_string(&path).expect("read");
        let mut script = Script::new(vec![
            Answer::Select(1),
            Answer::Input("feature".into()),
            Answer::Select(7),
        ]);
        edit_config(&mut config, &path, &mut script).expect("edit");
        assert_eq!(config.branch_template, "ticket/{ticket}");
        assert_eq!(fs::read_to_string(&path).expect("read"), before);
    }

    #[test]
    fn list_entries_can_be_added_and_removed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let added = tmp.path().join("more");
        let mut script = Script::new(vec![
            Answer::Select(3),                                   // repo_dirs
            Answer::Select(0),                                   // add
            Answer::Input(added.to_str().expect("utf8").into()), // entry
            Answer::Select(1),                                   // remove the first entry
            Answer::Select(2),                                   // back
            Answer::Select(7),                                   // exit
        ]);
        edit_config(&mut config, &path, &mut script).expect("edit");
        let reloaded = Config::load(Some(&path)).expect("reload");
        assert_eq!(reloaded.repo_dirs, vec![added]);
    }

    #[test]
    fn new_config_writes_only_the_answered_values() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("conf/config.yaml");
        let work = tmp.path().join("tickets");
        let code = tmp.path().join("code");
        let mut script = Script::new(vec![
            Answer::Input(String::new()), // work_dir is required
            Answer::Input(work.to_str().expect("utf8").into()), // work_dir
            Answer::Input(String::new()), // branch_template: default
            Answer::Input("main".into()), // base
            Answer::Select(0),            // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()), // repo dir
            Answer::Select(2),            // repo_dirs: back
            Answer::Input(String::new()), // clone_dir: default
            Answer::Select(1),            // repos: back
            Answer::Select(1),            // copy: back
        ]);
        new_config(&path, &mut script).expect("new config");

        let reloaded = Config::load(Some(&path)).expect("reload");
        assert_eq!(reloaded.work_dir, work);
        assert_eq!(reloaded.branch_template, "ticket/{ticket}");
        assert_eq!(reloaded.base, "main");
        assert_eq!(reloaded.repo_dirs, vec![code]);
        assert!(reloaded.repos.is_empty());
        let written = fs::read_to_string(&path).expect("read");
        assert!(!written.contains("repos:"), "empty lists are left out: {written}");
    }

    #[test]
    fn new_config_without_a_repo_source_writes_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.yaml");
        let work = tmp.path().join("tickets");
        let mut script = Script::new(vec![
            Answer::Input(work.to_str().expect("utf8").into()), // work_dir
            Answer::Input(String::new()),                       // branch_template
            Answer::Input(String::new()),                       // base
            Answer::Select(1),                                  // repo_dirs: back, skip
            Answer::Input(String::new()),                       // clone_dir
            Answer::Select(1),                                  // repos: back, skip
            Answer::Select(1),                                  // copy: back
        ]);
        let err = new_config(&path, &mut script).expect_err("no repo source");
        assert!(err.to_string().contains("repo_dirs"), "{err}");
        assert!(!path.exists(), "nothing written");
    }

    #[test]
    fn new_config_abort_writes_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.yaml");
        let mut script = Script::new(vec![Answer::Abort]);
        let err = new_config(&path, &mut script).expect_err("abort");
        assert!(err.to_string().contains("aborted"), "{err}");
        assert!(!path.exists(), "nothing written");
    }
}
