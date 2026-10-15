//! `youarel`: open web shortcuts from any browser, from a terminal launcher or from the command line.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use youarel_cli::commands::{self, Addition, Format};
use youarel_cli::{clock, home, sync, tui, update};
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
    /// The account this device syncs with.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Pair a new device through four files, which any channel carries.
    Pair {
        #[command(subcommand)]
        command: PairCommand,
    },
    /// Carry this device's state to the others as a file.
    Bundle {
        #[command(subcommand)]
        command: BundleCommand,
    },
    /// Sync through a shared folder, where each device writes its own bundle.
    Sync { folder: PathBuf },
    /// Remove a device from the account, by the start of its identifier.
    Remove { device: String },
    /// Replace this program with the latest release, when its signed manifest offers a newer one.
    SelfUpdate {
        /// Install the latest release even when it is older than the highest version this device reached.
        #[arg(long)]
        allow_downgrade: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AccountCommand {
    /// Found an account with this device as its only member.
    Create,
    /// Show the account, its members, and its epoch.
    Show,
}

#[derive(Debug, Subcommand)]
enum PairCommand {
    /// Show a code that a new device types to join.
    Invite,
    /// On the new device: answer a code with a join file.
    Join {
        code: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// On the inviting device: answer a join file with a welcome file.
    Accept {
        join: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// On the new device: answer a welcome file with a confirmation file.
    Confirm {
        welcome: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// On the inviting device: add the confirmed device and write its entry file.
    Admit {
        confirmation: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// On the new device: install an entry file.
    Finish { entry: PathBuf },
}

#[derive(Debug, Subcommand)]
enum BundleCommand {
    /// Write this device's state, sealed for the account's members.
    Export { out: PathBuf },
    /// Merge the state of another device's bundle.
    Import { file: PathBuf },
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
        Some(Command::Account { command }) => match command {
            AccountCommand::Create => sync::create(&store, now)?,
            AccountCommand::Show => sync::show(&store)?,
        },
        Some(Command::Pair { command }) => match command {
            PairCommand::Invite => sync::invite(&store)?,
            PairCommand::Join { code, out } => sync::join(&store, &code, &out)?,
            PairCommand::Accept { join, out } => sync::accept(&store, &join, &out)?,
            PairCommand::Confirm { welcome, out } => sync::confirm(&store, &welcome, &out)?,
            PairCommand::Admit { confirmation, out } => {
                sync::admit(&store, now, &confirmation, &out)?
            }
            PairCommand::Finish { entry } => sync::finish(&store, now, &entry)?,
        },
        Some(Command::Bundle { command }) => match command {
            BundleCommand::Export { out } => sync::export(&store, now, &out)?,
            BundleCommand::Import { file } => sync::import(&store, now, &file)?,
        },
        Some(Command::Sync { folder }) => sync::folder(&store, now, &folder)?,
        Some(Command::Remove { device }) => sync::remove(&store, now, &device)?,
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
