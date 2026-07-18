//! The `neuradix` binary entry point.
//!
//! Flow: parse (clap) -> dispatch to an application service -> wrap the result
//! in a versioned envelope -> render in the selected format -> exit with the
//! mapped code. Each stage lives in its own module so operation logic, parsing
//! and rendering stay separate.
#![forbid(unsafe_code)]

mod app;
mod cli;
mod envelope;
mod exit;
mod render;

use clap::Parser;

use app::{AppError, Outcome};
use cli::{
    Cli, Command, ContractCommand, ExplainCommand, GraphCommand, RecordCommand, ReplayCommand,
};
use envelope::CommandResult;
use exit::ExitCode;

fn main() {
    // `Cli::parse()` handles `--help`/`--version` (exit 0) and invalid usage
    // (exit 2) itself, matching the documented exit-code contract.
    let cli = Cli::parse();
    let format = cli.output;

    let (command_name, result) = dispatch(cli.command);

    let (envelope, code) = match result {
        Ok(Outcome { data, warnings }) => (
            CommandResult::success(&command_name, data).with_warnings(warnings),
            ExitCode::Success,
        ),
        Err(AppError { exit, errors, data }) => {
            let mut envelope = CommandResult::failure(&command_name, errors);
            envelope.data = data;
            (envelope, exit)
        }
    };

    println!("{}", render::render(&envelope, format));
    std::process::exit(code.code());
}

/// Route a parsed command to its application service, returning the dotted
/// command name and its result.
fn dispatch(command: Command) -> (String, Result<Outcome, AppError>) {
    match command {
        Command::Version => ("version".to_owned(), app::version::run()),
        Command::Doctor => ("doctor".to_owned(), app::doctor::run()),
        Command::Contract { command } => match command {
            ContractCommand::Validate { path } => (
                "contract.validate".to_owned(),
                app::contract::validate(&path),
            ),
            ContractCommand::Inspect { file } => {
                ("contract.inspect".to_owned(), app::contract::inspect(&file))
            }
            ContractCommand::Hash { file } => {
                ("contract.hash".to_owned(), app::contract::hash(&file))
            }
            ContractCommand::Generate {
                file,
                language,
                out_dir,
            } => (
                "contract.generate".to_owned(),
                app::contract::generate(&file, language, &out_dir),
            ),
        },
        Command::Record { command } => match command {
            RecordCommand::Inspect { file } => {
                ("record.inspect".to_owned(), app::record::inspect(&file))
            }
        },
        Command::Replay { command } => match command {
            ReplayCommand::Run {
                file,
                expect_digest,
            } => (
                "replay.run".to_owned(),
                app::record::replay_run(&file, expect_digest.as_deref()),
            ),
        },
        Command::Explain { command } => match command {
            ExplainCommand::Command { file, at } => (
                "explain.command".to_owned(),
                app::explain::command(&file, at),
            ),
        },
        Command::Graph { command } => match command {
            GraphCommand::Validate { file } => {
                ("graph.validate".to_owned(), app::graph::validate(&file))
            }
        },
    }
}
