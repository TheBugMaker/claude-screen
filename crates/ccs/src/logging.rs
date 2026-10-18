//! File logging. The terminal belongs to the TUI, so nothing is written to stdout/stderr.

use color_eyre::eyre::WrapErr;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt};

const LOG_FILE: &str = "ccs.log";

/// Starts logging to `<state_dir>/ccs.log`. Keep the guard alive until exit so logs are flushed.
pub fn init(filter: &str) -> color_eyre::Result<WorkerGuard> {
    let dir = ccs_core::paths::state_dir()?;
    std::fs::create_dir_all(&dir).wrap_err_with(|| format!("creating {}", dir.display()))?;

    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::never(dir, LOG_FILE));
    fmt()
        .with_env_filter(EnvFilter::try_new(filter).wrap_err("invalid log filter")?)
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Ok(guard)
}
