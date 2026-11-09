//! Interactive prompts via `inquire`; the only place with TUI code.

use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use grover_lib::config::{Config, CopySpec, RemoteRepo};
use grover_lib::error::Error;
use grover_lib::repo::{self, Source};
use inquire::ui::{Attributes, Color, ErrorMessageRenderConfig, RenderConfig, StyleSheet, Styled};
use inquire::{Confirm, InquireError, MultiSelect, Select, Text};

const PAGE_SIZE: usize = 12;
const INPUT_HELP: &str = "enter confirm · esc cancel";
const LIST_INPUT_HELP: &str = "enter confirm · esc back to the list";
const SELECT_HELP: &str = "↑↓ move · type filter · enter select · esc cancel";
const MULTI_SELECT_HELP: &str =
    "↑↓ move · space pick · →/← all/none · type filter · enter done · esc cancel";
const CONFIRM_HELP: &str = "enter confirm · typing y/n flips · ctrl-c abort";

/// Prompts only run on a terminal, so scripts and agents never hang.
pub fn available() -> bool {
    io::stdin().is_terminal()
}

/// What the flows ask; `None` means the user went back one step.
pub trait Prompter {
    fn input(&mut self, prompt: &str, default: &str, help: &str) -> anyhow::Result<Option<String>>;
    fn select(&mut self, prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>>;
    fn confirm(&mut self, prompt: &str, default: bool) -> anyhow::Result<bool>;
    fn multi_select(
        &mut self,
        prompt: &str,
        items: &[String],
    ) -> anyhow::Result<Option<Vec<usize>>>;
}

/// `inquire` on a real terminal: one theme, typing filters lists, stderr only.
pub struct Tui;

impl Prompter for Tui {
    fn input(&mut self, prompt: &str, default: &str, help: &str) -> anyhow::Result<Option<String>> {
        let answer = Text::new(prompt)
            .with_default(default)
            .with_help_message(help)
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

    fn confirm(&mut self, prompt: &str, default: bool) -> anyhow::Result<bool> {
        let answer = Confirm::new(prompt)
            .with_default(default)
            .with_help_message(CONFIRM_HELP)
            .with_render_config(theme())
            .prompt();
        match answer {
            Ok(answer) => Ok(answer),
            // Esc at the last question means "no": nothing is written either way.
            Err(InquireError::OperationCanceled) => Ok(false),
            Err(err) => Err(interrupted(err)),
        }
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
    theme.default_value = StyleSheet::new().with_fg(Color::DarkGrey);
    theme.placeholder = StyleSheet::new().with_fg(Color::DarkGrey);
    theme.answer = StyleSheet::new().with_fg(Color::LightCyan);
    theme.selected_option =
        Some(StyleSheet::new().with_fg(Color::LightCyan).with_attr(Attributes::BOLD));
    theme.error_message = ErrorMessageRenderConfig::default_colored();
    theme
}

/// ESC is "back one step", not an error; Ctrl-C is.
fn cancelled<T>(result: Result<Option<T>, InquireError>) -> anyhow::Result<Option<T>> {
    match result {
        Ok(answer) => Ok(answer),
        Err(InquireError::OperationCanceled) => Ok(None),
        Err(err) => Err(interrupted(err)),
    }
}

/// Ctrl-C (or any other prompt error) ends the run: exit 1, nothing written.
fn interrupted(err: InquireError) -> anyhow::Error {
    match err {
        InquireError::OperationInterrupted => anyhow::anyhow!("interrupted"),
        err => err.into(),
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
    prompter.input("Ticket", "", INPUT_HELP)?.ok_or_else(abort)
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

struct ScalarField {
    name: &'static str,
    help: &'static str,
    value: fn(&Config) -> String,
    set: fn(&mut Config, &str) -> anyhow::Result<()>,
}

static WORK_DIR: ScalarField = ScalarField {
    name: "work_dir",
    help: "Required; ticket folder parent, e.g. ~/work/tickets.",
    value: work_dir_value,
    set: set_work_dir,
};
static BRANCH_TEMPLATE: ScalarField = ScalarField {
    name: "branch_template",
    help: "Branch pattern, e.g. ticket/{ticket}; Enter keeps it.",
    value: |config| config.branch_template.clone(),
    set: set_branch_template,
};
static BASE: ScalarField = ScalarField {
    name: "base",
    help: "Fallback refs in order, e.g. main|master|HEAD; Enter keeps it.",
    value: |config| config.base.clone(),
    set: set_base,
};
static CLONE_DIR: ScalarField = ScalarField {
    name: "clone_dir",
    help: "Remote clone folder; empty clears it.",
    value: |config| config.clone_dir.as_deref().map(path_text).unwrap_or_default(),
    set: set_clone_dir,
};
static FIELDS: [&ScalarField; 4] = [&WORK_DIR, &BRANCH_TEMPLATE, &BASE, &CLONE_DIR];

/// One line of the editor menu, in the order the user sees it.
#[derive(Clone, Copy)]
enum Field {
    Scalar(&'static ScalarField),
    RepoDirs,
    Repos,
    Copy,
    Setup,
    Exit,
}

impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Scalar(field) => field.name,
            Self::RepoDirs => "repo_dirs",
            Self::Repos => "repos",
            Self::Copy => "copy",
            Self::Setup => "setup",
            Self::Exit => "",
        }
    }

    fn value(self, config: &Config) -> String {
        match self {
            Self::Scalar(field) => (field.value)(config),
            Self::RepoDirs => count(config.repo_dirs.len()),
            Self::Repos => count(config.repos.len()),
            Self::Copy => count(config.copy.len()),
            Self::Setup => count(config.setup.len()),
            Self::Exit => String::new(),
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Scalar(field) => field.help,
            Self::RepoDirs => "Search direct child folders for local repos.",
            Self::Repos => "Remote repos are cloned when selected.",
            Self::Copy => "Copy files into each ticket folder.",
            Self::Setup => "Run commands after each new worktree.",
            Self::Exit => "",
        }
    }
}

fn work_dir_value(config: &Config) -> String {
    path_text(&config.work_dir)
}

fn required_value(field: &str, value: &str) -> anyhow::Result<()> {
    if value.is_empty() {
        anyhow::bail!("{field} is required");
    }
    Ok(())
}

fn set_work_dir(config: &mut Config, value: &str) -> anyhow::Result<()> {
    required_value("work_dir", value)?;
    config.work_dir = PathBuf::from(value);
    Ok(())
}

fn set_branch_template(config: &mut Config, value: &str) -> anyhow::Result<()> {
    required_value("branch_template", value)?;
    config.branch_template = value.to_string();
    Ok(())
}

fn set_base(config: &mut Config, value: &str) -> anyhow::Result<()> {
    required_value("base", value)?;
    config.base = value.to_string();
    Ok(())
}

fn set_clone_dir(config: &mut Config, value: &str) -> anyhow::Result<()> {
    config.clone_dir = (!value.is_empty()).then(|| PathBuf::from(value));
    Ok(())
}

fn scalar_field(name: &str) -> Option<&'static ScalarField> {
    FIELDS.iter().copied().find(|field| field.name == name)
}

/// Config editor: one menu over all top-level fields, written after each entry.
/// Esc in the menu leaves the editor; Esc in a field goes back to the menu.
pub fn edit_config(
    config: &mut Config,
    path: &Path,
    prompter: &mut dyn Prompter,
) -> anyhow::Result<()> {
    loop {
        let fields = menu(config);
        let items: Vec<String> = fields.iter().map(|(_, label)| label.clone()).collect();
        let title = format!("Config · {}", path.display());
        let Some(choice) = prompter.select(&title, &items)? else {
            return Ok(());
        };
        // An index without a field leaves the editor: nothing to ask, nothing to write.
        let field = fields.get(choice).map_or(Field::Exit, |(field, _)| *field);
        // Lists write on leave: entries are already added or removed here.
        let changed = match field {
            Field::Scalar(field) => edit_scalar(prompter, config, field, field.name)?,
            Field::RepoDirs => {
                edit_list(
                    prompter,
                    "repo_dirs",
                    &mut config.repo_dirs,
                    |dir| path_text(dir),
                    ask_dir,
                )?;
                true
            }
            Field::Repos => {
                edit_list(prompter, "repos", &mut config.repos, remote_label, ask_remote)?;
                true
            }
            Field::Copy => {
                edit_list(prompter, "copy", &mut config.copy, copy_label, ask_copy)?;
                true
            }
            Field::Setup => {
                edit_list(
                    prompter,
                    "setup",
                    &mut config.setup,
                    |command| command.clone(),
                    ask_setup,
                )?;
                true
            }
            Field::Exit => return Ok(()),
        };
        if changed {
            if let Err(err) = save_and_reload(config, path) {
                tracing::warn!("not saved: {err}");
            }
        }
    }
}

const WIZARD_STEPS: usize = 8;

fn step_label(step: usize, name: &str) -> String {
    format!("[{step}/{WIZARD_STEPS}] {name}")
}

/// `config --init`: ask every field once, then write. Esc goes back one field,
/// Ctrl-C and a "no" in the last question leave no file behind.
pub fn new_config(path: &Path, prompter: &mut dyn Prompter) -> anyhow::Result<()> {
    let mut config = match Config::load(Some(path)) {
        Ok(existing) => existing,
        Err(err) => {
            // A broken file (only reachable with `--force`) starts empty instead.
            if path.exists() {
                tracing::warn!("{err}; starting from the defaults");
            }
            Config::new(PathBuf::new())
        }
    };
    let mut step = 1;
    while step <= WIZARD_STEPS {
        let forward = match step {
            1 => edit_scalar(prompter, &mut config, &WORK_DIR, &step_label(1, WORK_DIR.name))?,
            2 => edit_scalar(
                prompter,
                &mut config,
                &BRANCH_TEMPLATE,
                &step_label(2, BRANCH_TEMPLATE.name),
            )?,
            3 => edit_scalar(prompter, &mut config, &BASE, &step_label(3, BASE.name))?,
            4 => edit_list(
                prompter,
                &step_label(4, "repo_dirs"),
                &mut config.repo_dirs,
                |dir| path_text(dir),
                ask_dir,
            )?,
            5 => edit_scalar(prompter, &mut config, &CLONE_DIR, &step_label(5, CLONE_DIR.name))?,
            6 => edit_list(
                prompter,
                &step_label(6, "repos"),
                &mut config.repos,
                remote_label,
                ask_remote,
            )?,
            7 => {
                edit_list(prompter, &step_label(7, "copy"), &mut config.copy, copy_label, ask_copy)?
            }
            _ => edit_list(
                prompter,
                &step_label(8, "setup"),
                &mut config.setup,
                |command| command.clone(),
                ask_setup,
            )?,
        };
        if forward {
            step += 1;
        } else if step == 1 {
            return Err(abort());
        } else {
            step -= 1;
        }
    }
    config.validate()?;
    if !prompter.confirm(&format!("write {}?", path.display()), true)? {
        return Err(abort());
    }
    config.save(path)?;
    Ok(())
}

pub fn set_config_field(
    path: &Path,
    argument: &str,
    prompter: &mut dyn Prompter,
) -> anyhow::Result<()> {
    let (name, value) =
        argument.split_once('=').map_or((argument, None), |(name, value)| (name, Some(value)));
    let Some(field) = scalar_field(name) else {
        let fields = FIELDS.iter().map(|field| field.name).collect::<Vec<_>>().join(", ");
        anyhow::bail!("unknown field {name:?}; choose one of: {fields}");
    };
    let mut config = Config::load(Some(path))?;
    if let Some(value) = value {
        (field.set)(&mut config, value)?;
    } else {
        if !available() {
            anyhow::bail!("no terminal to ask for a value; use `--set {name}=<value>`");
        }
        loop {
            let answer = prompted_with_help(prompter, name, "", field.help)?;
            let value = answer.ok_or_else(abort)?;
            match (field.set)(&mut config, &value) {
                Ok(()) => break,
                Err(err) => tracing::warn!("{err}"),
            }
        }
    }
    save_and_reload(&mut config, path)
}

/// Labels of the editor menu, in the order of [`Field`]; the last entry leaves.
fn menu(config: &Config) -> Vec<(Field, String)> {
    [
        Field::Scalar(&WORK_DIR),
        Field::Scalar(&BRANCH_TEMPLATE),
        Field::Scalar(&BASE),
        Field::RepoDirs,
        Field::Scalar(&CLONE_DIR),
        Field::Repos,
        Field::Copy,
        Field::Setup,
        Field::Exit,
    ]
    .into_iter()
    .map(|field| {
        let label = if matches!(field, Field::Exit) {
            "← exit".to_string()
        } else {
            format!("{:<18}{:<24}{}", field.name(), field.value(config), field.help())
        };
        (field, label)
    })
    .collect()
}

fn count(entries: usize) -> String {
    match entries {
        1 => "1 entry".to_string(),
        entries => format!("{entries} entries"),
    }
}

/// Menu for one list field: add, remove, back. `false` means Esc went back a
/// step, `true` that the user left through `← back`.
fn edit_list<T>(
    prompter: &mut dyn Prompter,
    title: &str,
    items: &mut Vec<T>,
    label: impl Fn(&T) -> String,
    ask: impl Fn(&mut dyn Prompter) -> anyhow::Result<Option<T>>,
) -> anyhow::Result<bool> {
    loop {
        let mut entries = vec![format!("+ add {title}")];
        entries.extend(items.iter().map(&label));
        entries.push("← back".to_string());
        let Some(choice) = prompter.select(title, &entries)? else {
            return Ok(false);
        };
        if choice == 0 {
            if let Some(entry) = ask(prompter)? {
                items.push(entry);
            }
        } else if choice == entries.len() - 1 {
            return Ok(true);
        } else {
            items.remove(choice - 1);
        }
    }
}

/// One scalar prompt; `label` is the prompt and carries the wizard step.
/// Returns false when Esc went back a step, so the caller keeps the old value.
fn edit_scalar(
    prompter: &mut dyn Prompter,
    config: &mut Config,
    field: &'static ScalarField,
    label: &str,
) -> anyhow::Result<bool> {
    let current = (field.value)(config);
    loop {
        let Some(answer) = prompted_with_help(prompter, label, &current, field.help)? else {
            return Ok(false);
        };
        let value = if answer.is_empty() { &current } else { &answer };
        match (field.set)(config, value) {
            Ok(()) => return Ok(true),
            Err(err) => tracing::warn!("{err}"),
        }
    }
}

/// `None` means Esc: no entry, back to the list.
fn ask_dir(prompter: &mut dyn Prompter) -> anyhow::Result<Option<PathBuf>> {
    let answer = prompted(prompter, "repo dir", "")?;
    Ok(answer.filter(|answer| !answer.is_empty()).map(PathBuf::from))
}

fn ask_remote(prompter: &mut dyn Prompter) -> anyhow::Result<Option<RemoteRepo>> {
    let Some(url) = prompted(prompter, "repo url", "")? else {
        return Ok(None);
    };
    if url.is_empty() {
        return Ok(None);
    }
    let Some(name) = prompted(prompter, "name (empty = from url)", "")? else {
        return Ok(None);
    };
    Ok(Some(RemoteRepo { url, name: (!name.is_empty()).then_some(name) }))
}

fn ask_copy(prompter: &mut dyn Prompter) -> anyhow::Result<Option<CopySpec>> {
    let Some(from) = prompted(prompter, "from", "")? else {
        return Ok(None);
    };
    if from.is_empty() {
        return Ok(None);
    }
    let Some(to) = prompted(prompter, "to (empty = file name)", "")? else {
        return Ok(None);
    };
    let to = (!to.is_empty()).then(|| PathBuf::from(to));
    Ok(Some(CopySpec { from: PathBuf::from(from), to }))
}

fn ask_setup(prompter: &mut dyn Prompter) -> anyhow::Result<Option<String>> {
    let command = prompted(prompter, "command", "")?;
    Ok(command.filter(|command| !command.is_empty()))
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

fn prompted(
    prompter: &mut dyn Prompter,
    prompt: &str,
    default: &str,
) -> anyhow::Result<Option<String>> {
    prompted_with_help(prompter, prompt, default, LIST_INPUT_HELP)
}

fn prompted_with_help(
    prompter: &mut dyn Prompter,
    prompt: &str,
    default: &str,
    help: &str,
) -> anyhow::Result<Option<String>> {
    Ok(prompter.input(prompt, default, help)?.map(|answer| answer.trim().to_string()))
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
        Confirm(bool),
        Abort,
    }

    /// Scripted prompter: feeds answers and records every prompt.
    #[derive(Default)]
    struct Script {
        answers: VecDeque<Answer>,
        prompts: Vec<String>,
        help_messages: Vec<String>,
        defaults: Vec<String>,
    }

    impl Script {
        fn new(answers: Vec<Answer>) -> Script {
            Script {
                answers: answers.into(),
                prompts: Vec::new(),
                help_messages: Vec::new(),
                defaults: Vec::new(),
            }
        }

        fn next(&mut self, prompt: &str) -> Answer {
            self.prompts.push(prompt.to_string());
            self.answers.pop_front().expect("scripted answer")
        }
    }

    impl Prompter for Script {
        fn input(
            &mut self,
            prompt: &str,
            default: &str,
            help: &str,
        ) -> anyhow::Result<Option<String>> {
            self.help_messages.push(help.to_string());
            self.defaults.push(default.to_string());
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

        fn confirm(&mut self, prompt: &str, _default: bool) -> anyhow::Result<bool> {
            match self.next(prompt) {
                Answer::Confirm(answer) => Ok(answer),
                _ => panic!("scripted answer is not a confirm"),
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
    fn esc_goes_back_but_ctrl_c_aborts() {
        let cancelled_answer: Result<Option<u8>, InquireError> =
            Err(InquireError::OperationCanceled);
        assert_eq!(cancelled(cancelled_answer).expect("cancel"), None);
        let interrupted: Result<Option<u8>, InquireError> = Err(InquireError::OperationInterrupted);
        let err = cancelled(interrupted).expect_err("interrupt");
        assert!(err.to_string().contains("interrupted"), "{err}");
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
    fn scalar_fields_set_real_config_values() {
        let values = ["tickets", "ticket/{ticket}", "main", "clones"];
        let mut config = Config::new(PathBuf::from("old"));
        for (field, value) in FIELDS.iter().zip(values) {
            (field.set)(&mut config, value).expect("set field");
            let yaml = config.to_yaml().expect("yaml");
            assert!(
                yaml.lines().any(|line| line.starts_with(&format!("{}:", field.name))),
                "missing {} in {yaml}",
                field.name
            );
        }
    }

    #[test]
    fn menu_shows_field_help() {
        let config = Config::new(PathBuf::from("tickets"));
        let items = menu(&config);
        assert!(items[0].1.contains(WORK_DIR.help));
        assert!(items[3].1.contains(Field::RepoDirs.help()));
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
            Answer::Select(8),
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
            Answer::Select(8),
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
            Answer::Select(8),                                   // exit
        ]);
        edit_config(&mut config, &path, &mut script).expect("edit");
        let reloaded = Config::load(Some(&path)).expect("reload");
        assert_eq!(reloaded.repo_dirs, vec![added]);
    }

    #[test]
    fn setup_commands_can_be_added_in_the_editor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let mut script = Script::new(vec![
            Answer::Select(7),
            Answer::Select(0),
            Answer::Input("npm install".into()),
            Answer::Select(2),
            Answer::Select(8),
        ]);
        edit_config(&mut config, &path, &mut script).expect("edit");
        assert_eq!(config.setup, ["npm install"]);
        assert_eq!(Config::load(Some(&path)).expect("reload").setup, ["npm install"]);
    }

    #[test]
    fn new_config_writes_only_the_answered_values() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("conf/config.yaml");
        let work = tmp.path().join("tickets");
        let code = tmp.path().join("code");
        let mut script = Script::new(vec![
            Answer::Input(work.to_str().expect("utf8").into()), // work_dir
            Answer::Input(String::new()),                       // branch_template: default
            Answer::Input("main".into()),                       // base
            Answer::Select(0),                                  // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()), // repo dir
            Answer::Select(2),                                  // repo_dirs: back
            Answer::Input(String::new()),                       // clone_dir: default
            Answer::Select(1),                                  // repos: back
            Answer::Select(1),                                  // copy: back
            Answer::Select(0),                                  // setup: add
            Answer::Input("npm install".into()),
            Answer::Select(2),     // setup: back
            Answer::Confirm(true), // write?
        ]);
        new_config(&path, &mut script).expect("new config");
        assert!(script.help_messages.contains(&WORK_DIR.help.to_string()));
        assert!(script.help_messages.contains(&BRANCH_TEMPLATE.help.to_string()));

        let reloaded = Config::load(Some(&path)).expect("reload");
        assert_eq!(reloaded.work_dir, work);
        assert_eq!(reloaded.branch_template, "ticket/{ticket}");
        assert_eq!(reloaded.base, "main");
        assert_eq!(reloaded.repo_dirs, vec![code]);
        assert!(reloaded.repos.is_empty());
        assert_eq!(reloaded.setup, ["npm install"]);
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
            Answer::Select(1),                                  // setup: back
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

    #[test]
    fn esc_in_a_field_keeps_the_value_and_returns_to_the_menu() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let before = config.work_dir.clone();
        let mut script = Script::new(vec![
            Answer::Select(0), // work_dir
            Answer::Abort,     // esc in the field
            Answer::Select(8), // exit
        ]);
        edit_config(&mut config, &path, &mut script).expect("edit");
        assert_eq!(config.work_dir, before);
        assert_eq!(script.prompts[2], format!("Config · {}", path.display()));
    }

    #[test]
    fn esc_in_the_editor_menu_leaves_without_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let mut script = Script::new(vec![Answer::Abort]);
        edit_config(&mut config, &path, &mut script).expect("esc leaves the editor");
    }

    #[test]
    fn esc_in_the_wizard_goes_back_one_step() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.yaml");
        let work = tmp.path().join("tickets");
        let code = tmp.path().join("code");
        let mut script = Script::new(vec![
            Answer::Input(work.to_str().expect("utf8").into()), // work_dir
            Answer::Input(String::new()),                       // branch_template
            Answer::Abort,                                      // esc in base: back
            Answer::Input(String::new()),                       // branch_template again
            Answer::Input(String::new()),                       // base
            Answer::Select(0),                                  // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()),
            Answer::Select(2),            // repo_dirs: back
            Answer::Input(String::new()), // clone_dir
            Answer::Select(1),            // repos: back
            Answer::Select(1),            // copy: back
            Answer::Select(1),            // setup: back
            Answer::Confirm(true),        // write
        ]);
        new_config(&path, &mut script).expect("wizard");
        let reloaded = Config::load(Some(&path)).expect("reload");
        assert_eq!(reloaded.work_dir, work);
        assert_eq!(reloaded.repo_dirs, vec![code]);
        assert_eq!(script.prompts[2], "[3/8] base");
        assert_eq!(script.prompts[3], "[2/8] branch_template", "esc went back one step");
        assert_eq!(script.prompts[4], "[3/8] base");
    }

    #[test]
    fn esc_in_the_first_wizard_step_aborts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.yaml");
        let mut script = Script::new(vec![Answer::Abort]);
        let err = new_config(&path, &mut script).expect_err("abort");
        assert!(err.to_string().contains("aborted"), "{err}");
        assert!(!path.exists(), "nothing written");
    }

    #[test]
    fn no_at_the_final_question_writes_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.yaml");
        let work = tmp.path().join("tickets");
        let code = tmp.path().join("code");
        let mut script = Script::new(vec![
            Answer::Input(work.to_str().expect("utf8").into()), // work_dir
            Answer::Input(String::new()),                       // branch_template
            Answer::Input(String::new()),                       // base
            Answer::Select(0),                                  // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()),
            Answer::Select(2),            // repo_dirs: back
            Answer::Input(String::new()), // clone_dir
            Answer::Select(1),            // repos: back
            Answer::Select(1),            // copy: back
            Answer::Select(1),            // setup: back
            Answer::Confirm(false),       // do not write
        ]);
        let err = new_config(&path, &mut script).expect_err("declined");
        assert!(err.to_string().contains("aborted"), "{err}");
        assert!(!path.exists(), "nothing written");
    }

    #[test]
    fn wizard_defaults_show_the_existing_values() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, _) = config_file(tmp.path());
        let before = Config::load(Some(&path)).expect("load");
        let mut script = Script::new(vec![
            Answer::Input(before.work_dir.to_str().expect("utf8").into()),
            Answer::Input(String::new()),
            Answer::Input(String::new()),
            Answer::Select(2), // repo_dirs: back, keep the entry
            Answer::Input(String::new()),
            Answer::Select(1), // repos
            Answer::Select(1), // copy
            Answer::Select(1), // setup
            Answer::Confirm(true),
        ]);
        new_config(&path, &mut script).expect("wizard");
        assert!(script.defaults.contains(&before.branch_template));
        let reloaded = Config::load(Some(&path)).expect("reload");
        assert_eq!(reloaded.branch_template, before.branch_template);
        assert_eq!(reloaded.repo_dirs, before.repo_dirs);
    }

    #[test]
    fn ctrl_c_in_the_wizard_writes_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.yaml");
        let mut script = Script::new(vec![Answer::Abort]);
        let err = new_config(&path, &mut script).expect_err("abort");
        assert!(err.to_string().contains("aborted"), "{err}");
        assert!(!path.exists(), "nothing written");
    }

    #[test]
    fn a_broken_existing_file_starts_the_wizard_from_the_defaults() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.yaml");
        fs::write(&path, "work_dir: [oops\n").expect("broken config");
        let work = tmp.path().join("tickets");
        let code = tmp.path().join("code");
        let mut script = Script::new(vec![
            Answer::Input(work.to_str().expect("utf8").into()), // work_dir
            Answer::Input(String::new()),                       // branch_template: default
            Answer::Input(String::new()),                       // base: default
            Answer::Select(0),                                  // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()),
            Answer::Select(2),            // repo_dirs: back
            Answer::Input(String::new()), // clone_dir: default
            Answer::Select(1),            // repos: back
            Answer::Select(1),            // copy: back
            Answer::Select(1),            // setup: back
            Answer::Confirm(true),        // write
        ]);
        new_config(&path, &mut script).expect("wizard");
        assert_eq!(script.defaults[1], "ticket/{ticket}", "defaults, not the broken file");
        assert_eq!(Config::load(Some(&path)).expect("reload").base, "HEAD");
    }
}
