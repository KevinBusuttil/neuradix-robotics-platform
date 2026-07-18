//! Command-line parsing (the `clap` layer).
//!
//! This module only describes the command surface and produces a request model.
//! It performs no operation logic and no rendering.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::app::contract::Language;
use crate::render::OutputFormat;

/// The `neuradix` command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "neuradix",
    version,
    about = "Neuradix Robotics Platform command-line interface",
    long_about = None,
)]
pub struct Cli {
    /// Output format for machine or human consumption.
    #[arg(long, short = 'o', value_enum, default_value_t = OutputFormat::Table, global = true)]
    pub output: OutputFormat,

    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level commands implemented in this increment.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print version and build information.
    Version,

    /// Report development-environment diagnostics.
    Doctor,

    /// Work with Neuradix contracts.
    Contract {
        /// The contract subcommand.
        #[command(subcommand)]
        command: ContractCommand,
    },

    /// Work with recordings.
    Record {
        /// The record subcommand.
        #[command(subcommand)]
        command: RecordCommand,
    },

    /// Replay recordings.
    Replay {
        /// The replay subcommand.
        #[command(subcommand)]
        command: ReplayCommand,
    },

    /// Explain the causal lineage of a recorded command.
    Explain {
        /// The explain subcommand.
        #[command(subcommand)]
        command: ExplainCommand,
    },

    /// Work with deployment graphs.
    Graph {
        /// The graph subcommand.
        #[command(subcommand)]
        command: GraphCommand,
    },
}

/// `neuradix graph ...` subcommands.
#[derive(Debug, Subcommand)]
pub enum GraphCommand {
    /// Validate a deployment manifest's topology and policy offline.
    Validate {
        /// The deployment manifest file to validate.
        file: PathBuf,
    },
}

/// `neuradix explain ...` subcommands.
#[derive(Debug, Subcommand)]
pub enum ExplainCommand {
    /// Explain the causal chain of the command nearest a given time.
    Command {
        /// The recording file containing command lineage.
        file: PathBuf,

        /// The time of interest, in nanoseconds since the domain epoch.
        #[arg(long)]
        at: i128,
    },
}

/// `neuradix record ...` subcommands.
#[derive(Debug, Subcommand)]
pub enum RecordCommand {
    /// Show a recording's manifest, channels and replay digest.
    Inspect {
        /// The recording file to inspect.
        file: PathBuf,
    },
}

/// `neuradix replay ...` subcommands.
#[derive(Debug, Subcommand)]
pub enum ReplayCommand {
    /// Replay a recording and report its deterministic replay digest.
    Run {
        /// The recording file to replay.
        file: PathBuf,

        /// Fail with the determinism exit code if the replay digest differs.
        #[arg(long = "expect-digest")]
        expect_digest: Option<String>,
    },
}

/// `neuradix contract ...` subcommands.
#[derive(Debug, Subcommand)]
pub enum ContractCommand {
    /// Validate a contract file, or every contract under a directory.
    Validate {
        /// A contract file or a directory to search recursively.
        path: PathBuf,
    },

    /// Show the parsed contents and schema identity of a contract.
    Inspect {
        /// The contract file to inspect.
        file: PathBuf,
    },

    /// Print the content-addressed schema identity of a contract.
    Hash {
        /// The contract file to hash.
        file: PathBuf,
    },

    /// Generate a typed representation of a contract.
    Generate {
        /// The contract file to generate from.
        file: PathBuf,

        /// The target language.
        #[arg(long, value_enum, default_value_t = Language::Rust)]
        language: Language,

        /// The directory to write generated code into.
        ///
        /// Named `--out-dir` because the global `--output` selects the
        /// machine-readable result format (see RFC-0013).
        #[arg(long = "out-dir")]
        out_dir: PathBuf,
    },
}
