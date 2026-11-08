//! `youarel`: open web shortcuts from any browser, from a terminal launcher or from the command line.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::process::ExitCode;
use youarel_cli::commands::{self, Addition};
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
    Ls {
        folder: Option<String>,
        /// List at most this many entries, the first ones.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Search the shortcuts and folders.
    Find {
        #[arg(required = true)]
        query: Vec<String>,
        /// List at most this many matches, the best ones.
        #[arg(long)]
        limit: Option<usize>,
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
    /// Write a shortcut's note, or clear it with an empty one.
    Note { entry: String, note: String },
    /// Give a folder a mark to show in place of its glyph, such as one emoji, or clear it with an empty one.
    Mark { folder: String, mark: String },
    /// Remove a shortcut, or a folder with everything in it.
    Rm { entry: String },
    /// Print the directory that holds youarel's files.
    Where,
    /// Show the window launcher's window, or hide it, for a desktop shortcut where the launcher cannot hold its own.
    Toggle,
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
        Some(Command::Ls { folder, limit }) => {
            commands::list(&store, now, folder.as_deref(), limit.unwrap_or(usize::MAX))?
        }
        Some(Command::Find { query, limit }) => {
            commands::find(&store, now, &joined(&query), limit.unwrap_or(usize::MAX))?
        }
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
        Some(Command::Note { entry, note }) => vec![commands::note(&store, now, &entry, &note)?],
        Some(Command::Mark { folder, mark }) => vec![commands::mark(&store, now, &folder, &mark)?],
        Some(Command::Rm { entry }) => vec![commands::remove(&store, now, &entry)?],
        Some(Command::Where) => vec![store.directory().display().to_string()],
        Some(Command::Toggle) => vec![youarel_cli::window::toggle()?],
        Some(Command::SelfUpdate { allow_downgrade }) => {
            vec![update::run(store.directory(), consent(allow_downgrade))?]
        }
    };
    commands::print(&lines)
}

/// What a person allows an update to do.
const fn consent(allow_downgrade: bool) -> Consent {
    if allow_downgrade {
        Consent::Downgrade
    } else {
        Consent::Forward
    }
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

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::{CommandFactory as _, Parser as _};

    #[test]
    fn the_command_surface_contains_only_bookmark_window_and_update_operations() {
        let command = Cli::command();
        let names: Vec<&str> = command
            .get_subcommands()
            .map(clap::Command::get_name)
            .collect();
        assert_eq!(
            names,
            [
                "add",
                "ls",
                "find",
                "open",
                "mkdir",
                "mv",
                "rename",
                "alias",
                "note",
                "mark",
                "rm",
                "where",
                "toggle",
                "self-update",
            ]
        );
    }

    #[test]
    fn retired_account_and_transport_commands_are_rejected() {
        for arguments in [
            vec!["account", "show"],
            vec!["pair", "invite"],
            vec!["bundle", "export", "unused"],
            vec!["sync", "unused"],
            vec!["remove", "abc"],
            vec!["recovery", "create"],
            vec!["recover", "--bundle", "unused"],
            vec!["import", "unused.html"],
            vec!["export"],
            vec!["icons"],
        ] {
            let error =
                Cli::try_parse_from(core::iter::once("youarel").chain(arguments)).unwrap_err();
            assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
        }
    }
}
