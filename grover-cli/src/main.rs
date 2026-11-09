//! CLI `grover`: arguments, output, exit code.

mod tui;

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::anyhow;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use grover_lib::config::Config;
use grover_lib::plan::{self, Plan, Request, existing_tickets};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "grover", version, about = "Ticket worktrees across many repositories")]
struct Cli {
    /// Config file; defaults to ./grover.yaml, else the XDG path
    #[arg(short, long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    /// More output: `-v` debug, `-vv` trace
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Warnings and errors only
    #[arg(short, long, global = true)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the ticket directory with worktrees; the directory is the output
    New {
        /// Ticket, e.g. PROJ-123; asked on a terminal when omitted
        ticket: Option<String>,
        /// Repo names, fuzzy; repeated or comma separated; asked when omitted
        #[arg(short, long, value_name = "NAME")]
        repo: Vec<String>,
        /// Every known repo
        #[arg(short, long, conflicts_with = "repo")]
        all: bool,
        /// Ticket title, e.g. "Login redirect loop"; needs --slug-from-title
        #[arg(short = 't', long, value_name = "TEXT")]
        title: Option<String>,
        /// Put a slug of the title into the branch name
        #[arg(long, requires = "title")]
        slug_from_title: bool,
        /// Show only, write nothing
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// List work items (ticket directories)
    List,
    /// Remove a ticket: worktrees, branches, copies, then the directory itself
    Rm {
        /// Ticket, e.g. PROJ-123; picked on a terminal when omitted
        ticket: Option<String>,
        /// Remove dirty worktrees and unmerged branches too
        #[arg(short, long)]
        force: bool,
        /// Show only, remove nothing
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Print the directory of a ticket
    Path {
        /// Ticket, e.g. PROJ-123; the ticket above the cwd when omitted
        ticket: Option<String>,
    },
    /// Show branch, upstream distance and dirty files of every worktree
    Status {
        /// Ticket, e.g. PROJ-123; the ticket above the cwd when omitted
        ticket: Option<String>,
    },
    /// Edit the config interactively on a terminal
    Config {
        /// Set one scalar field; prompt for the value when omitted
        #[arg(long, value_name = "FIELD[=VALUE]", conflicts_with_all = ["path", "show", "init"])]
        set: Option<String>,
        /// Show the config location only
        #[arg(short, long, conflicts_with = "show")]
        path: bool,
        /// Print the resolved config
        #[arg(short = 's', long)]
        show: bool,
        /// Ask for the values and write a config; an existing config is the target
        #[arg(short, long, conflicts_with_all = ["path", "show"])]
        init: bool,
        /// Overwrite an existing config
        #[arg(long, requires = "init")]
        force: bool,
    },
    /// Print a completion script
    Completions { shell: Shell },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose, cli.quiet);
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let config_path = cli.config.as_deref();
    match cli.command {
        Command::New { ticket, repo, all, title, slug_from_title, dry_run } => {
            let config = Config::load(config_path)?;
            let (ticket, repos) = new_inputs(&config, ticket, repo, all)?;
            let request = Request { ticket, repos, all, title, slug_from_title };
            let plan = Plan::build(&config, &request)?;
            let report = plan.apply(dry_run)?;
            if dry_run {
                tracing::info!("dry-run: nothing written");
            }
            tracing::info!(
                created = report.created.len(),
                skipped = report.skipped.len(),
                cloned = report.cloned.len(),
                "done"
            );
            println!("{}", plan.ticket_dir.display());
        }
        Command::List => {
            for ticket in existing_tickets(&Config::load(config_path)?)? {
                println!("{ticket}");
            }
        }
        Command::Rm { ticket, force, dry_run } => {
            let config = Config::load(config_path)?;
            let ticket = rm_ticket(&config, ticket)?;
            let ticket_dir = plan::resolve_ticket(&config, &ticket)?;
            plan::remove(&config, &ticket_dir, force, dry_run)?;
            if dry_run {
                tracing::info!("dry-run: nothing removed");
            }
        }
        Command::Path { ticket } => {
            let config = Config::load(config_path)?;
            println!("{}", ticket_dir(&config, ticket.as_deref())?.display());
        }
        Command::Status { ticket } => {
            let config = Config::load(config_path)?;
            let dir = ticket_dir(&config, ticket.as_deref())?;
            print_status(&plan::statuses(&dir)?);
        }
        Command::Config { path, show, init, force, set } => {
            config_cmd(config_path, path, show, init, force, set.as_deref())?
        }
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "grover", &mut io::stdout());
        }
    }
    Ok(())
}

/// Ticket and repos: the arguments, else a prompt on a terminal, else the old error.
fn new_inputs(
    config: &Config,
    ticket: Option<String>,
    repos: Vec<String>,
    all: bool,
) -> anyhow::Result<(String, Vec<String>)> {
    if tui::available() {
        return tui::complete(&mut tui::Tui, config, ticket, repos, all);
    }
    let ticket = ticket.ok_or_else(|| anyhow!("missing ticket: pass <ticket>"))?;
    Ok((ticket, repos))
}

/// Ticket for `rm`: the argument, else a pick on a terminal; never guessed from the cwd.
fn rm_ticket(config: &Config, ticket: Option<String>) -> anyhow::Result<String> {
    if let Some(ticket) = ticket {
        return Ok(ticket);
    }
    if !tui::available() {
        anyhow::bail!("missing ticket: pass <ticket>");
    }
    let tickets = existing_tickets(config)?;
    if tickets.is_empty() {
        anyhow::bail!("no tickets in {}", config.work_dir.display());
    }
    tui::pick_ticket(&mut tui::Tui, &tickets)
}

/// Ticket directory for a read-only command: the argument, else the ticket above the cwd.
fn ticket_dir(config: &Config, ticket: Option<&str>) -> anyhow::Result<PathBuf> {
    match ticket {
        Some(ticket) => Ok(plan::find_ticket(config, ticket)?),
        None => Ok(plan::ticket_above(config, &std::env::current_dir()?)?),
    }
}

/// The `status` table: header plus one row per worktree, aligned, no trailing spaces.
fn print_status(rows: &[plan::RepoStatus]) {
    let mut table: Vec<[String; 4]> =
        vec![["repo".into(), "branch".into(), "ahead/behind".into(), "dirty".into()]];
    for row in rows {
        table.push([
            row.name.clone(),
            row.branch.clone().unwrap_or_else(|| "HEAD".to_string()),
            match (row.ahead, row.behind) {
                (Some(ahead), Some(behind)) => format!("+{ahead}/-{behind}"),
                _ => "-".to_string(),
            },
            match row.dirty {
                0 => "clean".to_string(),
                1 => "1 file".to_string(),
                files => format!("{files} files"),
            },
        ]);
    }
    let mut widths = [0; 3];
    for row in &table {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    for row in &table {
        let mut line = String::new();
        for (index, cell) in row[..3].iter().enumerate() {
            let width = widths[index];
            line.push_str(&format!("{cell:<width$}  "));
        }
        line.push_str(&row[3]);
        println!("{line}");
    }
}

/// `config`: the location, the resolved YAML, the editor, or a new config from prompts.
fn config_cmd(
    config_path: Option<&Path>,
    path: bool,
    show: bool,
    init: bool,
    force: bool,
    set: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(argument) = set {
        let target = Config::locate(config_path)?;
        tui::set_config_field(&target, argument, &mut tui::Tui)?;
        tracing::info!("updated {}", target.display());
        return Ok(());
    }
    if init {
        let target = Config::target(config_path)?;
        if target.exists() && !force {
            anyhow::bail!("{} exists, use `--force` to overwrite", target.display());
        }
        if !tui::available() {
            anyhow::bail!("no terminal to ask for the values; write {} yourself", target.display());
        }
        tui::new_config(&target, &mut tui::Tui)?;
        tracing::info!("wrote {}; `grover config` edits it", target.display());
        return Ok(());
    }
    let located = Config::locate(config_path)?;
    if path {
        println!("{}", located.display());
    } else if show {
        print!("{}", Config::load(Some(&located))?.to_yaml()?);
    } else if !tui::available() {
        anyhow::bail!("no terminal for the editor; use `grover config --show`");
    } else {
        let mut config = Config::load(Some(&located))?;
        tui::edit_config(&mut config, &located, &mut tui::Tui)?;
    }
    Ok(())
}

fn init_logging(verbose: u8, quiet: bool) {
    let level = match (quiet, verbose) {
        (true, _) => "warn",
        (false, 0) => "info",
        (false, 1) => "debug",
        (false, _) => "trace",
    };
    let filter = EnvFilter::try_from_env("GROVER_LOG").unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .without_time()
        .with_target(false)
        .with_ansi(false)
        .init();
}
