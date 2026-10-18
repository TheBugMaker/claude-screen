//! Domain model and configuration for claude-screen.
//!
//! Owns the session/group model, the `[[tab]]` definitions (commands, parameters,
//! modes) and config/state paths. Has no I/O beyond reading config files.

pub mod config;
pub mod keys;
pub mod paths;

/// Errors produced by the core crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not determine the home directory")]
    NoHomeDir(#[from] etcetera::HomeDirError),
    #[error("reading the config failed")]
    Read(#[source] std::io::Error),
    #[error("invalid config")]
    Parse(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}
