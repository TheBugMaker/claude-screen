//! Where claude-screen keeps its files.
//!
//! XDG layout on both Linux and macOS (`~/.config/ccs`, `~/.local/state/ccs`),
//! which is what terminal users expect on macOS too.

use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

use crate::Error;

const APP_DIR: &str = "ccs";

/// Directory holding `config.toml`.
pub fn config_dir() -> Result<PathBuf, Error> {
    Ok(etcetera::choose_base_strategy()?.config_dir().join(APP_DIR))
}

/// The config of the project whose root is `root`: `.ccs/config.toml` there.
#[must_use]
pub fn project_config(root: &Path) -> PathBuf {
    root.join(".ccs").join("config.toml")
}

/// Directory for logs and persisted session state.
pub fn state_dir() -> Result<PathBuf, Error> {
    let strategy = etcetera::choose_base_strategy()?;
    let base = strategy.state_dir().unwrap_or_else(|| strategy.data_dir());
    Ok(base.join(APP_DIR))
}

/// The Unix socket this ccs listens on for the reports of its Claude sessions: `ccs-<pid>.sock`
/// in the runtime directory, else the state directory.
pub fn socket_path() -> Result<PathBuf, Error> {
    Ok(runtime_dir()?.join(format!("ccs-{}.sock", std::process::id())))
}

/// The plugin this ccs writes to give its Claude sessions the theme: `ccs-<pid>-theme` next to
/// the socket.
pub fn theme_plugin_dir() -> Result<PathBuf, Error> {
    Ok(runtime_dir()?.join(format!("ccs-{}-theme", std::process::id())))
}

/// Where the files of this run go: the runtime directory, else the state directory.
fn runtime_dir() -> Result<PathBuf, Error> {
    match etcetera::choose_base_strategy()?.runtime_dir() {
        Some(dir) => Ok(dir),
        None => state_dir(),
    }
}
