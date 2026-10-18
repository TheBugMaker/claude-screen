use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// A terminal workspace for running and navigating many Claude Code sessions.
#[derive(Debug, Parser)]
#[command(name = "ccs", version, about, args_conflicts_with_subcommands = true)]
pub struct Cli {
    /// Directory to start the first Claude session in.
    #[arg(default_value = ".")]
    pub dir: PathBuf,

    /// Resume a past Claude Code session by id (runs in that session's directory).
    #[arg(long, value_name = "SESSION_ID", conflicts_with = "dir")]
    pub resume: Option<String>,

    /// Log filter written to the log file (e.g. `info`, `debug`, `ccs_claude=trace`).
    #[arg(long, env = "CCS_LOG", default_value = "info")]
    pub log_level: String,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sends the report on stdin to the ccs listening on `socket`, for session `session`. Run
    /// by the ccs-bridge mod.
    #[command(hide = true)]
    Hook { socket: PathBuf, session: u64 },
}
