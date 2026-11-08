//! CLI `grover`: arguments, output, exit code.

mod tui;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::anyhow;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use grover_lib::config::Config;
use grover_lib::plan::{Plan, Request, existing_tickets};
use tracing_subscriber::EnvFilter;

/// Written by `grover init`.
const EXAMPLE: &str = include_str!("../../grover.yaml.example");

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
        /// Show only, write nothing
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// List work items (ticket directories)
    List,
    /// Write the example config; never overwrites without --force
    Init {
        /// Overwrite an existing file
        #[arg(short, long)]
        force: bool,
    },
    /// Edit the config interactively on a terminal
    Config {
        /// Show the config location only
        #[arg(short, long, conflicts_with = "show")]
        path: bool,
        /// Print the resolved config
        #[arg(short = 's', long)]
        show: bool,
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
        Command::New { ticket, repo, all, dry_run } => {
            let config = Config::load(config_path)?;
            let (ticket, repos) = new_inputs(&config, ticket, repo, all)?;
            let plan = Plan::build(&config, &Request { ticket, repos, all })?;
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
        Command::Init { force } => init(config_path, force)?,
        Command::Config { path, show } => config_cmd(config_path, path, show)?,
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

/// `init`: write the example config, creating missing directories.
fn init(config_path: Option<&Path>, force: bool) -> anyhow::Result<()> {
    let path = Config::target(config_path)?;
    if path.exists() && !force {
        anyhow::bail!("{} exists, use `--force` to overwrite", path.display());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, EXAMPLE)?;
    tracing::info!("wrote {}; edit it with `grover config`", path.display());
    Ok(())
}

/// `config`: the location, the resolved YAML, or the interactive editor.
fn config_cmd(config_path: Option<&Path>, path: bool, show: bool) -> anyhow::Result<()> {
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
        .init();
}
