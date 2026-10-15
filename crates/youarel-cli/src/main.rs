//! `youarel`: open web shortcuts from any browser, from a terminal launcher or from the command line.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use youarel_cli::commands::{self, Addition, Format};
use youarel_cli::{clock, home, tui, update};
use youarel_core::update_rules::Consent;
use youarel_store::Store;

/// Open your web shortcuts from any browser; without a command, the terminal launcher opens.
#[derive(Debug, Parser)]
#[command(name = "youarel", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add a shortcut, titled by its page unless a title is given.
    Add {
        link: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        alias: Option<String>,
        /// The folder path, such as `Work/Infra`.
        #[arg(long)]
        folder: Option<String>,
        /// Keep the link as the title instead of fetching the page.
        #[arg(long)]
        no_fetch: bool,
    },
    /// List the entries of a folder, or of the top.
    Ls { folder: Option<String> },
    /// Search the shortcuts and folders.
    Find {
        #[arg(required = true)]
        query: Vec<String>,
    },
    /// Open the best shortcut for a search in the default browser.
    Open {
        #[arg(required = true)]
        query: Vec<String>,
    },
    /// Create a folder and the folders above it that do not exist.
    Mkdir { path: String },
    /// Move a shortcut or a folder to the end of a folder; `/` is the top.
    Mv { entry: String, folder: String },
    /// Give a shortcut a new title, or a folder a new name.
    Rename { entry: String, name: String },
    /// Give a shortcut an alias that finds it first, or remove it with an empty one.
    Alias { entry: String, alias: String },
    /// Remove a shortcut, or a folder with everything in it.
    Rm { entry: String },
    /// Import a bookmark file a browser exported.
    Import { file: PathBuf },
    /// Export every folder and shortcut.
    Export {
        #[arg(long, value_enum, default_value = "html")]
        format: Format,
    },
    /// Print the directory that holds youarel's files.
    Where,
    /// Replace this program with the latest release, when its signed manifest offers a newer one.
    SelfUpdate {
        /// Install the latest release even when it is older than the highest version this device reached.
        #[arg(long)]
        allow_downgrade: bool,
    },
}

fn joined(words: &[String]) -> String {
    words.join(" ")
}

fn run(cli: Cli) -> Result<()> {
    let store = Store::new(home::data_directory()?);
    let now = clock::now()?;
    let lines = match cli.command {
        None => return tui::run(&store),
        Some(Command::Add {
            link,
            title,
            alias,
            folder,
            no_fetch,
        }) => {
            let addition = Addition {
                title,
                alias,
                folder,
                fetch: !no_fetch,
            };
            vec![commands::add(&store, now, &link, &addition)?]
        }
        Some(Command::Ls { folder }) => commands::list(&store, now, folder.as_deref())?,
        Some(Command::Find { query }) => commands::find(&store, now, &joined(&query))?,
        Some(Command::Open { query }) => vec![commands::open(&store, now, &joined(&query))?],
        Some(Command::Mkdir { path }) => vec![commands::make_folder(&store, now, &path)?],
        Some(Command::Mv { entry, folder }) => {
            vec![commands::move_entry(&store, now, &entry, &folder)?]
        }
        Some(Command::Rename { entry, name }) => {
            vec![commands::rename(&store, now, &entry, &name)?]
        }
        Some(Command::Alias { entry, alias }) => {
            vec![commands::alias(&store, now, &entry, &alias)?]
        }
        Some(Command::Rm { entry }) => vec![commands::remove(&store, now, &entry)?],
        Some(Command::Import { file }) => {
            let text = read_file(&file)?;
            let counts = commands::import(&store, now, &text)?;
            vec![format!(
                "imported {} shortcuts and {} folders; {} were already here, and {} were no web links",
                counts.shortcuts, counts.folders, counts.duplicates, counts.refused
            )]
        }
        Some(Command::Export { format }) => {
            let text = commands::export(&store, now, format)?;
            vec![text.trim_end().to_owned()]
        }
        Some(Command::Where) => vec![store.directory().display().to_string()],
        Some(Command::SelfUpdate { allow_downgrade }) => {
            let consent = if allow_downgrade {
                Consent::Downgrade
            } else {
                Consent::Forward
            };
            vec![update::run(store.directory(), consent)?]
        }
    };
    commands::print(&lines)
}

/// The text of a file the person named.
fn read_file(path: &std::path::Path) -> Result<String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the binary reads the one file a person names for import here, and nowhere else"
    )]
    let bytes = std::fs::read(path);
    let bytes = bytes.with_context(|| format!("cannot read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("youarel: {error:#}");
            ExitCode::FAILURE
        }
    }
}
