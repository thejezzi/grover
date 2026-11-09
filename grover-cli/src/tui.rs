//! Interactive prompts via `inquire`; the only place with TUI code.

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::{self, ClearType};
use grover_lib::config::{Config, CopySpec, RemoteRepo};
use grover_lib::error::Error;
use grover_lib::repo::{self, Source};
use inquire::ui::{Attributes, Color, ErrorMessageRenderConfig, RenderConfig, StyleSheet, Styled};
use inquire::{Autocomplete, Confirm, CustomUserError, InquireError};

const PAGE_SIZE: usize = 12;
const MAX_SUGGESTIONS: usize = 1000;
const INPUT_HELP: &str = "enter confirm · esc cancel";
const LIST_INPUT_HELP: &str = "enter confirm · esc back to the list";
const PATH_INPUT_HELP: &str = "tab complete · ↑↓ pick a suggestion · enter confirm · esc back";
const LIST_HELP: &str = "j/k move · / search · enter pick · esc back";
const MULTI_LIST_HELP: &str =
    "j/k move · space pick · h/l none/all · / search · enter done · esc back";
const CONFIRM_HELP: &str = "enter confirm · typing y/n flips · ctrl-c abort";

/// Prompts only run on a terminal, so scripts and agents never hang.
pub fn available() -> bool {
    io::stdin().is_terminal()
}

/// What the flows ask; `None` means the user went back one step.
pub trait Prompter {
    fn input(&mut self, prompt: &str, default: &str, help: &str) -> anyhow::Result<Option<String>>;
    /// Like [`Prompter::input`], with tab completion for paths; `files` also
    /// suggests plain files, otherwise only folders.
    fn input_path(
        &mut self,
        prompt: &str,
        default: &str,
        help: &str,
        files: bool,
    ) -> anyhow::Result<Option<String>>;
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
        let answer = inquire::Text::new(prompt)
            .with_default(default)
            .with_help_message(help)
            .with_render_config(theme())
            .prompt_skippable();
        cancelled(answer)
    }

    fn input_path(
        &mut self,
        prompt: &str,
        default: &str,
        help: &str,
        files: bool,
    ) -> anyhow::Result<Option<String>> {
        let answer = inquire::Text::new(prompt)
            .with_default(default)
            .with_help_message(help)
            .with_autocomplete(PathCompleter::new(files))
            .with_page_size(PAGE_SIZE)
            .with_render_config(theme())
            .prompt_skippable();
        cancelled(answer)
    }

    fn select(&mut self, prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>> {
        Ok(pick(prompt, items, false)?.and_then(|picked| picked.first().copied()))
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
        pick(prompt, items, true)
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

/// Raw mode while the picker runs; dropping the guard restores the terminal.
struct RawMode;

impl RawMode {
    fn new() -> anyhow::Result<RawMode> {
        terminal::enable_raw_mode()?;
        Ok(RawMode)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// What one key did to the picker.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Keep,
    Submit,
    Back,
    /// Ctrl-C: the whole run stops, nothing is written.
    Interrupt,
}

/// List state: `j`/`k` (or arrows) move, `/` searches, space toggles, Enter picks.
struct Picker<'a> {
    prompt: &'a str,
    items: &'a [String],
    multi: bool,
    picks: Vec<bool>,
    query: String,
    searching: bool,
    /// Index into the filtered list, not into `items`.
    cursor: usize,
}

impl<'a> Picker<'a> {
    fn new(prompt: &'a str, items: &'a [String], multi: bool) -> Picker<'a> {
        Picker {
            prompt,
            items,
            multi,
            picks: vec![false; items.len()],
            query: String::new(),
            searching: false,
            cursor: 0,
        }
    }

    /// Indices of `items` that contain the query, case-insensitive.
    fn matched(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..self.items.len()).collect();
        }
        let needle = self.query.to_lowercase();
        (0..self.items.len())
            .filter(|&index| self.items[index].to_lowercase().contains(&needle))
            .collect()
    }

    fn handle(&mut self, key: KeyEvent) -> Step {
        if key.code == KeyCode::Char('c')
            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return Step::Interrupt;
        }
        if self.searching {
            match key.code {
                KeyCode::Enter => self.searching = false,
                KeyCode::Esc => {
                    self.query.clear();
                    self.searching = false;
                    self.cursor = 0;
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.cursor = 0;
                }
                KeyCode::Char(char) => {
                    self.query.push(char);
                    self.cursor = 0;
                }
                _ => {}
            }
            return Step::Keep;
        }
        match key.code {
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('j') | KeyCode::Down => self.cursor += 1,
            KeyCode::Char('k') | KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => self.cursor = 0,
            KeyCode::Char('G') | KeyCode::End => self.cursor = usize::MAX,
            KeyCode::Char('h') if self.multi => self.picks.fill(false),
            KeyCode::Char('l') if self.multi => self.picks.fill(true),
            KeyCode::Char(' ') if self.multi => {
                if let Some(index) = self.matched().get(self.cursor) {
                    self.picks[*index] = !self.picks[*index];
                }
            }
            KeyCode::Enter => {
                // Enter on an empty result set would pick nothing; stay instead.
                if self.matched().get(self.cursor).is_some() {
                    return Step::Submit;
                }
            }
            KeyCode::Esc => return Step::Back,
            _ => {}
        }
        // Keep the cursor inside the filtered list after a move or a filter change.
        self.cursor = self.cursor.min(self.matched().len().saturating_sub(1));
        Step::Keep
    }

    /// The answer: the entry under the cursor, or every picked entry in list order.
    fn answer(&self) -> Vec<usize> {
        if self.multi {
            return (0..self.items.len()).filter(|&index| self.picks[index]).collect();
        }
        self.matched().get(self.cursor).copied().into_iter().collect()
    }

    fn lines(&self) -> Vec<String> {
        let matched = self.matched();
        let mut lines = vec![if self.query.is_empty() {
            format!("? {}", self.prompt)
        } else {
            format!("? {}  /{}", self.prompt, self.query)
        }];
        if matched.is_empty() {
            lines.push("  no matches".to_string());
        }
        // Window around the cursor, so a long list stays readable.
        let first = self.cursor.saturating_sub(PAGE_SIZE / 2);
        for (position, &index) in matched.iter().enumerate().skip(first).take(PAGE_SIZE) {
            let mark = if position == self.cursor { "❯" } else { " " };
            let entry = if self.multi {
                let box_mark = if self.picks[index] { "[✓]" } else { "[ ]" };
                format!("{mark} {box_mark} {}", self.items[index])
            } else {
                format!("{mark} {}", self.items[index])
            };
            lines.push(entry);
        }
        let help = if self.searching {
            "type to search · enter keep filter · esc clear"
        } else if self.multi {
            MULTI_LIST_HELP
        } else {
            LIST_HELP
        };
        lines.push(help.to_string());
        lines
    }
}

/// One list prompt on the terminal; `None` means Esc.
fn pick(prompt: &str, items: &[String], multi: bool) -> anyhow::Result<Option<Vec<usize>>> {
    if items.is_empty() {
        anyhow::bail!("{prompt}: nothing to pick from");
    }
    let _raw = RawMode::new()?;
    let mut out = io::stderr();
    let mut picker = Picker::new(prompt, items, multi);
    let mut drawn = 0usize;
    loop {
        render_picker(&mut out, &picker.lines(), &mut drawn)?;
        let Event::Key(key) = event::read()? else { continue };
        // Windows reports press and release; only a press counts.
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match picker.handle(key) {
            Step::Keep => {}
            Step::Back => return Ok(None),
            Step::Interrupt => anyhow::bail!("interrupted"),
            Step::Submit => {
                // Leave the answer visible as the last frame instead of erasing it.
                render_picker(&mut out, &picker.lines(), &mut drawn)?;
                return Ok(Some(picker.answer()));
            }
        }
    }
}

/// Draw `lines` where the previous frame was; `drawn` carries the frame height,
/// so a shorter frame wipes the lines it no longer uses.
fn render_picker(out: &mut impl Write, lines: &[String], drawn: &mut usize) -> io::Result<()> {
    if *drawn > 0 {
        queue!(out, cursor::MoveUp(*drawn as u16))?;
    }
    let previous = *drawn;
    for line in lines {
        queue!(out, terminal::Clear(ClearType::CurrentLine), Print(line), Print("\r\n"))?;
    }
    for _ in lines.len()..previous {
        queue!(out, terminal::Clear(ClearType::CurrentLine), Print("\r\n"))?;
    }
    *drawn = lines.len();
    out.flush()
}

/// Tab completion for paths: the parent directory is listed, names are matched
/// against the last segment. A typed `~` stays a `~` in the suggestions.
#[derive(Clone)]
struct PathCompleter {
    cwd: PathBuf,
    /// `false` for folder prompts: only directories are suggested.
    files: bool,
}

impl PathCompleter {
    fn new(files: bool) -> PathCompleter {
        PathCompleter { cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")), files }
    }

    /// Typed prefix (through the last `/`), the directory to read, and the
    /// partial last segment. A lone `~` lists the home directory.
    fn split(&self, input: &str) -> (String, PathBuf, String) {
        match input.rfind('/') {
            Some(index) => {
                let prefix = &input[..=index];
                (prefix.to_string(), self.lookup(prefix), input[index + 1..].to_string())
            }
            None if input == "~" => ("~/".to_string(), self.lookup("~/"), String::new()),
            None => (String::new(), self.cwd.clone(), input.to_string()),
        }
    }

    /// The typed prefix as a real directory to read; `~` only exists as text.
    fn lookup(&self, prefix: &str) -> PathBuf {
        if prefix.is_empty() {
            return self.cwd.clone();
        }
        if let Some(rest) = prefix.strip_prefix('~') {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest.trim_start_matches('/'));
            }
        }
        let path = PathBuf::from(prefix);
        if path.is_absolute() { path } else { self.cwd.join(path) }
    }

    /// Candidate entries for `input`, as they would be shown to the user.
    fn suggestions(&self, input: &str) -> Vec<String> {
        let (prefix, dir, partial) = self.split(input);
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| self.files || entry.path().is_dir())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&partial))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names.truncate(MAX_SUGGESTIONS);
        names.into_iter().map(|name| format!("{prefix}{name}")).collect()
    }
}

impl Autocomplete for PathCompleter {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        Ok(self.suggestions(input))
    }

    fn get_completion(
        &mut self,
        input: &str,
        highlighted: Option<String>,
    ) -> Result<Option<String>, CustomUserError> {
        if let Some(suggestion) = highlighted {
            return Ok(Some(suggestion));
        }
        let suggestions = self.suggestions(input);
        let Some(first) = suggestions.first() else {
            return Ok(None);
        };
        let prefix = suggestions.iter().fold(first.clone(), |common, next| {
            common.chars().zip(next.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect()
        });
        Ok((prefix.chars().count() > input.chars().count()).then_some(prefix))
    }
}

/// Fills `new` inputs: prompts only for what is missing.
pub fn complete(
    prompter: &mut dyn Prompter,
    config: &Config,
    ticket: Option<String>,
    repos: Vec<String>,
    all: bool,
    title: Option<String>,
) -> anyhow::Result<(String, Vec<String>, Option<String>)> {
    let ticket = match ticket {
        Some(ticket) => ticket,
        None => ask_ticket(prompter)?,
    };
    // Only worth asking when a branch will carry the title.
    let title = match title {
        Some(title) => Some(title),
        None if config.create_branch => ask_title(prompter)?,
        None => None,
    };
    if all || !repos.is_empty() {
        return Ok((ticket, repos, title));
    }
    Ok((ticket, ask_repos(prompter, config)?, title))
}

/// Ticket for `new`; the slug check happens later in `Plan::build`.
fn ask_ticket(prompter: &mut dyn Prompter) -> anyhow::Result<String> {
    prompter.input("Ticket", "", INPUT_HELP)?.ok_or_else(abort)
}

/// Optional title for the branch name; empty means none.
fn ask_title(prompter: &mut dyn Prompter) -> anyhow::Result<Option<String>> {
    let answer = prompter.input("Title (empty = none)", "", INPUT_HELP)?.ok_or_else(abort)?;
    Ok((!answer.trim().is_empty()).then(|| answer.trim().to_string()))
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
    /// `Some(true)` for file paths, `Some(false)` for folders, `None` for text.
    path: Option<bool>,
    value: fn(&Config) -> String,
    set: fn(&mut Config, &str) -> anyhow::Result<()>,
}

static WORK_DIR: ScalarField = ScalarField {
    name: "work_dir",
    help: "Required; ticket folder parent, e.g. ~/work/tickets.",
    path: Some(false),
    value: work_dir_value,
    set: set_work_dir,
};
static BRANCH_TEMPLATE: ScalarField = ScalarField {
    name: "branch_template",
    help: "Branch pattern, e.g. ticket/{ticket}; Enter keeps it.",
    path: None,
    value: |config| config.branch_template.clone(),
    set: set_branch_template,
};
static BASE: ScalarField = ScalarField {
    name: "base",
    help: "Fallback refs in order, e.g. main|master|HEAD; Enter keeps it.",
    path: None,
    value: |config| config.base.clone(),
    set: set_base,
};
static CREATE_BRANCH: ScalarField = ScalarField {
    name: "create_branch",
    help: "true: create the ticket branch in each worktree; false: detached.",
    path: None,
    value: |config| config.create_branch.to_string(),
    set: set_create_branch,
};
static CLONE_DIR: ScalarField = ScalarField {
    name: "clone_dir",
    help: "Clone folder for remotes; empty means no clones.",
    path: Some(false),
    value: |config| config.clone_dir.as_deref().map(path_text).unwrap_or_default(),
    set: set_clone_dir,
};
static FIELDS: [&ScalarField; 5] = [&WORK_DIR, &BRANCH_TEMPLATE, &BASE, &CREATE_BRANCH, &CLONE_DIR];

/// One line of the editor menu, in the order the user sees it.
#[derive(Clone, Copy)]
enum Field {
    Scalar(&'static ScalarField),
    RepoDirs,
    Repos,
    Discover,
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
            Self::Discover => "discover",
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
            Self::Discover => count(config.discover.len()),
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
            Self::Discover => "Commands printing clone URLs, one per line.",
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

fn set_create_branch(config: &mut Config, value: &str) -> anyhow::Result<()> {
    config.create_branch = match value.trim().to_lowercase().as_str() {
        "true" | "yes" | "1" => true,
        "false" | "no" | "0" => false,
        _ => anyhow::bail!("create_branch is true or false, not {value:?}"),
    };
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
            Field::Discover => {
                edit_list(
                    prompter,
                    "discover",
                    &mut config.discover,
                    |command| command.clone(),
                    ask_discover,
                )?;
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

const WIZARD_STEPS: usize = 9;

fn step_label(step: usize, name: &str) -> String {
    format!("[{step}/{WIZARD_STEPS}] {name}")
}

/// `config --init`: ask every field once, then write. Esc goes back one field,
/// Ctrl-C and a "no" in the last question leave no file behind.
/// `clone_dir` is not asked: it stays empty until set by hand or with `--set`.
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
            4 => edit_scalar(
                prompter,
                &mut config,
                &CREATE_BRANCH,
                &step_label(4, CREATE_BRANCH.name),
            )?,
            5 => edit_list(
                prompter,
                &step_label(5, "repo_dirs"),
                &mut config.repo_dirs,
                |dir| path_text(dir),
                ask_dir,
            )?,
            6 => edit_list(
                prompter,
                &step_label(6, "repos"),
                &mut config.repos,
                remote_label,
                ask_remote,
            )?,
            7 => edit_list(
                prompter,
                &step_label(7, "discover"),
                &mut config.discover,
                |command| command.clone(),
                ask_discover,
            )?,
            8 => {
                edit_list(prompter, &step_label(8, "copy"), &mut config.copy, copy_label, ask_copy)?
            }
            _ => edit_list(
                prompter,
                &step_label(9, "setup"),
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
            let answer = prompt_value(prompter, field, name, "")?;
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
        Field::Scalar(&CREATE_BRANCH),
        Field::RepoDirs,
        Field::Scalar(&CLONE_DIR),
        Field::Repos,
        Field::Discover,
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
        let answer = prompt_value(prompter, field, label, &current)?;
        let Some(answer) = answer else {
            return Ok(false);
        };
        let value = if answer.is_empty() { &current } else { &answer };
        match (field.set)(config, value) {
            Ok(()) => return Ok(true),
            Err(err) => tracing::warn!("{err}"),
        }
    }
}

/// One text answer for a scalar field; path fields offer tab completion.
fn prompt_value(
    prompter: &mut dyn Prompter,
    field: &ScalarField,
    prompt: &str,
    default: &str,
) -> anyhow::Result<Option<String>> {
    let answer = match field.path {
        Some(files) => prompter.input_path(prompt, default, field.help, files)?,
        None => prompter.input(prompt, default, field.help)?,
    };
    Ok(answer.map(|answer| answer.trim().to_string()))
}

/// `None` means Esc: no entry, back to the list.
fn ask_dir(prompter: &mut dyn Prompter) -> anyhow::Result<Option<PathBuf>> {
    let answer = prompted_path(prompter, "repo dir", "", false)?;
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
    let Some(from) = prompted_path(prompter, "from", "", true)? else {
        return Ok(None);
    };
    if from.is_empty() {
        return Ok(None);
    }
    // `to` is relative to the ticket folder, which does not exist yet: no completion.
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

fn ask_discover(prompter: &mut dyn Prompter) -> anyhow::Result<Option<String>> {
    let command = prompted(prompter, "discover command", "")?;
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

/// A path prompt with tab completion; `None` is Esc.
fn prompted_path(
    prompter: &mut dyn Prompter,
    prompt: &str,
    default: &str,
    files: bool,
) -> anyhow::Result<Option<String>> {
    let answer = prompter.input_path(prompt, default, PATH_INPUT_HELP, files)?;
    Ok(answer.map(|answer| answer.trim().to_string()))
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
        path_prompts: Vec<String>,
    }

    impl Script {
        fn new(answers: Vec<Answer>) -> Script {
            Script {
                answers: answers.into(),
                prompts: Vec::new(),
                help_messages: Vec::new(),
                defaults: Vec::new(),
                path_prompts: Vec::new(),
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

        fn input_path(
            &mut self,
            prompt: &str,
            default: &str,
            help: &str,
            _files: bool,
        ) -> anyhow::Result<Option<String>> {
            self.path_prompts.push(prompt.to_string());
            self.input(prompt, default, help)
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
            "clone_dir: clones\n",
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
        let values = ["tickets", "ticket/{ticket}", "main", "true", "clones"];
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
        assert!(items[4].1.contains(Field::RepoDirs.help()));
        assert!(items[3].1.contains(CREATE_BRANCH.help));
    }

    /// Temporary tree with `alpha/`, `alpha-beta/` and a plain file.
    fn completer_tree() -> (tempfile::TempDir, PathCompleter) {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(tmp.path().join("alpha")).expect("alpha");
        fs::create_dir(tmp.path().join("alpha-beta")).expect("alpha-beta");
        fs::write(tmp.path().join("alphafile"), "").expect("alphafile");
        fs::create_dir(tmp.path().join("other")).expect("other");
        let completer = PathCompleter { cwd: tmp.path().to_path_buf(), files: true };
        (tmp, completer)
    }

    #[test]
    fn suggestions_match_the_last_segment() {
        let (tmp, mut completer) = completer_tree();
        let prefix = format!("{}/alpha", tmp.path().display());
        let mut suggestions = completer.get_suggestions(&prefix).expect("suggestions");
        suggestions.sort();
        let expected: Vec<String> = ["alpha", "alpha-beta", "alphafile"]
            .iter()
            .map(|name| format!("{}/{}", tmp.path().display(), name))
            .collect();
        assert_eq!(suggestions, expected);
    }

    #[test]
    fn suggestions_of_a_bare_prefix_are_relative() {
        let (_tmp, mut completer) = completer_tree();
        let suggestions = completer.get_suggestions("alp").expect("suggestions");
        assert_eq!(suggestions, ["alpha", "alpha-beta", "alphafile"]);
    }

    #[test]
    fn folder_prompts_skip_plain_files() {
        let (tmp, completer) = completer_tree();
        let mut folders = PathCompleter { cwd: completer.cwd.clone(), files: false };
        assert_eq!(folders.get_suggestions("alp").expect("suggestions"), ["alpha", "alpha-beta"]);
        assert!(folders.get_suggestions("alphafile").expect("suggestions").is_empty());
        assert!(tmp.path().join("alphafile").is_file());
    }

    #[test]
    fn a_prefix_without_matches_has_no_suggestions() {
        let (_tmp, mut completer) = completer_tree();
        assert!(completer.get_suggestions("zzz").expect("suggestions").is_empty());
        assert_eq!(completer.get_completion("zzz", None).expect("completion"), None);
    }

    #[test]
    fn completion_takes_the_longest_common_prefix() {
        let (tmp, mut completer) = completer_tree();
        let prefix = format!("{}/alph", tmp.path().display());
        let completion = completer.get_completion(&prefix, None).expect("completion");
        assert_eq!(completion, Some(format!("{}/alpha", tmp.path().display())));
    }

    #[test]
    fn a_highlighted_suggestion_wins() {
        let (_tmp, mut completer) = completer_tree();
        let picked = Some("alpha-beta".to_string());
        assert_eq!(
            completer.get_completion("al", picked).expect("completion"),
            Some("alpha-beta".into())
        );
    }

    #[test]
    fn completion_never_shortens_the_input() {
        let (_tmp, mut completer) = completer_tree();
        assert_eq!(completer.get_completion("alpha", None).expect("completion"), None);
    }

    #[test]
    fn tilde_stays_a_tilde_in_the_suggestions() {
        let mut completer = PathCompleter::new(false);
        let suggestions = completer.get_suggestions("~/.gro").expect("suggestions");
        for suggestion in &suggestions {
            assert!(suggestion.starts_with("~/"), "kept the shortcut: {suggestion}");
        }
    }

    #[test]
    fn a_lone_tilde_lists_the_home_directory() {
        let mut completer = PathCompleter::new(false);
        let suggestions = completer.get_suggestions("~").expect("suggestions");
        let home = dirs::home_dir().expect("home");
        let expected: Option<String> = fs::read_dir(&home).ok().and_then(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .min()
                .map(|name| format!("~/{name}"))
        });
        assert_eq!(suggestions.first().cloned(), expected);
    }

    #[test]
    fn path_fields_get_the_completing_prompt() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let elsewhere = tmp.path().join("elsewhere");
        let mut script = Script::new(vec![
            Answer::Select(0), // work_dir
            Answer::Input(elsewhere.to_str().expect("utf8").into()),
            Answer::Select(4), // repo_dirs
            Answer::Select(0), // add
            Answer::Input(tmp.path().to_str().expect("utf8").into()),
            Answer::Select(3),  // back (two entries plus add and back)
            Answer::Select(10), // exit
        ]);
        edit_config(&mut config, &path, &mut script).expect("edit");
        assert_eq!(script.path_prompts, ["work_dir", "repo dir"]);
    }

    #[test]
    fn non_path_fields_keep_the_plain_prompt() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let mut script = Script::new(vec![
            Answer::Select(2), // base
            Answer::Input("develop".into()),
            Answer::Select(10),
        ]);
        edit_config(&mut config, &path, &mut script).expect("edit");
        assert!(script.path_prompts.is_empty(), "base is not a path");
        assert_eq!(config.base, "develop");
    }

    /// Keys as the picker sees them; only `code` matters here.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn entries() -> Vec<String> {
        ["api", "api-gateway", "web"].iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn j_and_k_move_and_stop_at_the_ends() {
        let items = entries();
        let mut picker = Picker::new("Repos", &items, false);
        picker.handle(key(KeyCode::Char('k')));
        assert_eq!(picker.cursor, 0, "k at the top stays");
        picker.handle(key(KeyCode::Char('j')));
        assert_eq!(picker.cursor, 1);
        for _ in 0..9 {
            picker.handle(key(KeyCode::Char('j')));
        }
        assert_eq!(picker.cursor, 2, "j at the bottom stays");
        picker.handle(key(KeyCode::Char('g')));
        assert_eq!(picker.cursor, 0);
    }

    #[test]
    fn typing_without_slash_does_not_search() {
        let items = entries();
        let mut picker = Picker::new("Repos", &items, false);
        picker.handle(key(KeyCode::Char('w')));
        assert_eq!(picker.query, "", "letters are not a filter");
        assert_eq!(picker.matched().len(), 3);
    }

    #[test]
    fn slash_starts_the_search_and_enter_keeps_it() {
        let items = entries();
        let mut picker = Picker::new("Repos", &items, false);
        picker.handle(key(KeyCode::Char('/')));
        for char in "gate".chars() {
            picker.handle(key(KeyCode::Char(char)));
        }
        assert_eq!(picker.matched(), [1]);
        assert_eq!(picker.answer(), [1]);
        picker.handle(key(KeyCode::Enter));
        assert!(!picker.searching);
        assert_eq!(picker.query, "gate", "the filter stays after enter");
        assert_eq!(picker.lines()[0], "? Repos  /gate");
    }

    #[test]
    fn esc_in_the_search_clears_the_query() {
        let items = entries();
        let mut picker = Picker::new("Repos", &items, false);
        picker.handle(key(KeyCode::Char('/')));
        picker.handle(key(KeyCode::Char('w')));
        assert_eq!(picker.handle(key(KeyCode::Esc)), Step::Keep);
        assert_eq!(picker.query, "");
        assert!(!picker.searching);
        assert_eq!(picker.matched().len(), 3);
    }

    #[test]
    fn a_search_without_matches_cannot_be_submitted() {
        let items = entries();
        let mut picker = Picker::new("Repos", &items, false);
        picker.handle(key(KeyCode::Char('/')));
        picker.handle(key(KeyCode::Char('z')));
        picker.handle(key(KeyCode::Enter));
        assert_eq!(picker.handle(key(KeyCode::Enter)), Step::Keep, "no empty answer");
        assert_eq!(picker.answer(), Vec::<usize>::new());
        assert!(picker.lines().iter().any(|line| line.contains("no matches")));
    }

    #[test]
    fn multi_select_toggles_with_space_and_takes_list_order() {
        let items = entries();
        let mut picker = Picker::new("Repos", &items, true);
        picker.handle(key(KeyCode::Char('j')));
        picker.handle(key(KeyCode::Char(' ')));
        picker.handle(key(KeyCode::Char('g')));
        picker.handle(key(KeyCode::Char(' ')));
        assert_eq!(picker.answer(), [0, 1], "picked in list order, not in click order");
        picker.handle(key(KeyCode::Char(' ')));
        assert_eq!(picker.answer(), [1], "space toggles off");
        picker.handle(key(KeyCode::Char('h')));
        assert_eq!(picker.answer(), Vec::<usize>::new());
        picker.handle(key(KeyCode::Char('l')));
        assert_eq!(picker.answer(), [0, 1, 2]);
    }

    #[test]
    fn esc_and_enter_leave_the_picker() {
        let items = entries();
        let mut picker = Picker::new("Repos", &items, true);
        assert_eq!(picker.handle(key(KeyCode::Esc)), Step::Back);
        assert_eq!(picker.handle(key(KeyCode::Enter)), Step::Submit);
    }

    #[test]
    fn the_lines_window_follows_the_cursor() {
        let items: Vec<String> = (0..40).map(|index| format!("repo-{index:02}")).collect();
        let mut picker = Picker::new("Repos", &items, false);
        picker.cursor = 30;
        let lines = picker.lines();
        assert!(lines.len() <= PAGE_SIZE + 2, "{lines:?}");
        assert!(lines.iter().any(|line| line.contains("❯ repo-30")), "{lines:?}");
    }

    #[test]
    fn ctrl_c_interrupts_the_picker() {
        let items = entries();
        let mut picker = Picker::new("Repos", &items, true);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), crossterm::event::KeyModifiers::CONTROL);
        assert_eq!(picker.handle(ctrl_c), Step::Interrupt);
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
        let (ticket, repos, title) =
            complete(&mut script, &config, given.0, given.1, false, None).expect("complete");
        assert_eq!(ticket, "PROJ-1");
        assert_eq!(repos, ["api"]);
        assert_eq!(title, None);
        assert!(script.prompts.is_empty());
    }

    #[test]
    fn all_skips_the_repo_prompt() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("remotes.yaml");
        let mut script = Script::new(vec![Answer::Input("PROJ-2".into())]);
        let (ticket, repos, _) =
            complete(&mut script, &remote_config(&path), None, vec![], true, None)
                .expect("complete");
        assert_eq!(ticket, "PROJ-2");
        assert!(repos.is_empty());
        assert_eq!(script.prompts, ["Ticket"]);
    }

    #[test]
    fn a_title_is_asked_only_when_branches_are_created() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("remotes.yaml");
        let mut branchy = remote_config(&path);
        branchy.create_branch = true;
        let mut script =
            Script::new(vec![Answer::Input("PROJ-3".into()), Answer::Input("Dark mode".into())]);
        let (_, _, title) =
            complete(&mut script, &branchy, None, vec![], true, None).expect("complete");
        assert_eq!(title.as_deref(), Some("Dark mode"));
        assert_eq!(script.prompts, ["Ticket", "Title (empty = none)"]);

        // Without branches the title has no effect, so the prompt stays away.
        let mut plain = remote_config(&path);
        plain.create_branch = false;
        let mut script = Script::new(vec![Answer::Input("PROJ-4".into())]);
        let (_, _, title) =
            complete(&mut script, &plain, None, vec![], true, None).expect("complete");
        assert_eq!(title, None);
        assert_eq!(script.prompts, ["Ticket"]);
    }

    #[test]
    fn an_empty_title_answer_means_no_title() {
        let mut script = Script::new(vec![Answer::Input(String::new())]);
        assert_eq!(ask_title(&mut script).expect("title"), None);

        let mut given = Script::new(vec![Answer::Input("  Login  ".into())]);
        assert_eq!(ask_title(&mut given).expect("title").as_deref(), Some("Login"));
    }

    #[test]
    fn editing_a_value_writes_and_reloads() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (path, mut config) = config_file(tmp.path());
        let elsewhere = tmp.path().join("elsewhere");
        let mut script = Script::new(vec![
            Answer::Select(0),
            Answer::Input(elsewhere.to_str().expect("utf8").into()),
            Answer::Select(10),
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
            Answer::Select(10),
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
            Answer::Select(4),                                   // repo_dirs
            Answer::Select(0),                                   // add
            Answer::Input(added.to_str().expect("utf8").into()), // entry
            Answer::Select(1),                                   // remove the first entry
            Answer::Select(2),                                   // back
            Answer::Select(10),                                  // exit
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
            Answer::Select(9),
            Answer::Select(0),
            Answer::Input("npm install".into()),
            Answer::Select(2),
            Answer::Select(10),
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
            Answer::Input("true".into()),                       // create_branch
            Answer::Select(0),                                  // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()), // repo dir
            Answer::Select(2),                                  // repo_dirs: back
            Answer::Select(1),                                  // repos: back
            Answer::Select(1),                                  // discover: back
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
        assert!(reloaded.create_branch);
        assert!(reloaded.repos.is_empty());
        assert!(reloaded.clone_dir.is_none(), "the wizard does not ask for clone_dir");
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
            Answer::Input(String::new()),                       // create_branch
            Answer::Select(1),                                  // repo_dirs: back, skip
            Answer::Select(1),                                  // repos: back, skip
            Answer::Select(1),                                  // discover: back
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
            Answer::Select(0),  // work_dir
            Answer::Abort,      // esc in the field
            Answer::Select(10), // exit
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
            Answer::Input(String::new()),                       // create_branch
            Answer::Select(0),                                  // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()),
            Answer::Select(2),     // repo_dirs: back
            Answer::Select(1),     // repos: back
            Answer::Select(1),     // discover: back
            Answer::Select(1),     // copy: back
            Answer::Select(1),     // setup: back
            Answer::Confirm(true), // write
        ]);
        new_config(&path, &mut script).expect("wizard");
        let reloaded = Config::load(Some(&path)).expect("reload");
        assert_eq!(reloaded.work_dir, work);
        assert_eq!(reloaded.repo_dirs, vec![code]);
        assert_eq!(script.prompts[2], "[3/9] base");
        assert_eq!(script.prompts[3], "[2/9] branch_template", "esc went back one step");
        assert_eq!(script.prompts[4], "[3/9] base");
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
            Answer::Input(String::new()),                       // create_branch
            Answer::Select(0),                                  // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()),
            Answer::Select(2),      // repo_dirs: back
            Answer::Select(1),      // repos: back
            Answer::Select(1),      // discover: back
            Answer::Select(1),      // copy: back
            Answer::Select(1),      // setup: back
            Answer::Confirm(false), // do not write
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
            Answer::Input(String::new()), // create_branch: default false
            Answer::Select(2),            // repo_dirs: back, keep the entry
            Answer::Select(1),            // repos
            Answer::Select(1),            // discover
            Answer::Select(1),            // copy
            Answer::Select(1),            // setup
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
            Answer::Input(String::new()),                       // create_branch: default
            Answer::Select(0),                                  // repo_dirs: add
            Answer::Input(code.to_str().expect("utf8").into()),
            Answer::Select(2),     // repo_dirs: back
            Answer::Select(1),     // repos: back
            Answer::Select(1),     // discover: back
            Answer::Select(1),     // copy: back
            Answer::Select(1),     // setup: back
            Answer::Confirm(true), // write
        ]);
        new_config(&path, &mut script).expect("wizard");
        assert_eq!(script.defaults[1], "ticket/{ticket}", "defaults, not the broken file");
        assert_eq!(Config::load(Some(&path)).expect("reload").base, "HEAD");
    }
}
