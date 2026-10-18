//! Claude Code sessions as ccs sees them.
//!
//! Each session is an interactive `claude` in a terminal, where the ccs-bridge mod reports what
//! it does (`bridge`, applied by `session`). Past sessions are read from Claude Code's own files
//! (`history`).

pub mod bridge;
pub mod history;
pub mod session;

/// Errors produced while finding Claude Code's files.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not determine the home directory")]
    NoHomeDir(#[from] etcetera::HomeDirError),
}
