//! PTY-backed terminal tabs.
//!
//! Runs a command in a pseudo-terminal (`portable-pty`), keeps its screen in a
//! `vt100` parser and renders it with `tui-term`. Also encodes key events into
//! the bytes a terminal program expects.

pub mod keys;
pub mod pty;

pub use pty::{EventSender, Pty, PtyEvent};

/// Errors produced by terminal tabs.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("opening a terminal failed: {0}")]
    Pty(String),
    #[error("starting the command failed: {0}")]
    Spawn(String),
}
