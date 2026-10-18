//! `ccs`: a terminal workspace for running and navigating many Claude Code sessions.

mod app;
mod cli;
mod finder;
mod git;
mod hook;
mod input;
mod keys;
mod logging;
mod merge;
mod socket;
mod theme;
mod ui;

use std::io::stdout;

use ccs_core::{config::Config, paths};
use clap::Parser;
use color_eyre::eyre::{WrapErr, eyre};
use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    style::Print,
};
use tokio::sync::mpsc;

use crate::{
    app::App,
    cli::{Cli, Command},
    keys::KeyMatch,
};

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    if let Some(Command::Hook { socket, session }) = &cli.command {
        hook::run(socket, *session);
        return Ok(());
    }
    let _log_guard = logging::init(&cli.log_level)?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

    // Read the config and start the first session before taking over the terminal, so errors
    // print normally.
    let config_dir = paths::config_dir()?;
    let config_path = config_dir.join("config.toml");
    let config = Config::load(&config_path)
        .wrap_err_with(|| format!("loading {}", config_path.display()))?;
    let leader = KeyMatch::parse(&config.leader)
        .map_err(|err| eyre!("{}: leader: {err}", config_path.display()))?;
    let claude_theme = match &config.theme {
        Some(name) => {
            let (colors, text) = theme::load(name, &config_dir.join("themes"))
                .wrap_err_with(|| format!("{}: theme", config_path.display()))?;
            theme::install(colors);
            let dir = paths::theme_plugin_dir()?;
            let plugin = theme::ClaudeTheme::write(dir.clone(), name, &text)
                .wrap_err_with(|| format!("writing {}", dir.display()))?;
            Some(plugin)
        }
        None => None,
    };
    let (scans, scanned) = mpsc::unbounded_channel();
    let (ptys, pty_events) = mpsc::unbounded_channel();
    let (reports, bridge_reports) = mpsc::unbounded_channel();
    let socket_path = paths::socket_path()?;
    let socket = socket::listen(socket_path.clone(), reports)
        .wrap_err_with(|| format!("listening on {}", socket_path.display()))?;
    let mut app = App::new(
        config,
        leader,
        scans,
        ptys,
        socket.path().to_owned(),
        claude_theme,
    );
    match &cli.resume {
        Some(id) => app.resume(id)?,
        None => app.open(&cli.dir, None, true)?,
    }

    let mut terminal = ratatui::init();
    // ratatui's panic hook restores raw mode and the main screen, but not the modes set below.
    let ratatui_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave_modes();
        ratatui_hook(info);
    }));
    // Without bracketed paste, a pasted newline would submit the prompt.
    execute!(stdout(), EnableBracketedPaste)?;
    // Mouse buttons in SGR encoding, for the wheel. Not EnableMouseCapture: its motion tracking
    // would redraw on every mouse move. Shift+drag still selects text in most terminals.
    execute!(stdout(), Print("\x1b[?1000h\x1b[?1006h"))?;
    // Without the kitty keyboard protocol, Ctrl+digit arrives as a plain digit (or not at all).
    // Terminals that don't support it ignore this.
    execute!(
        stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;
    let result = app
        .run(&mut terminal, scanned, pty_events, bridge_reports)
        .await;
    leave_modes();
    ratatui::restore();
    result
}

/// Turns off the terminal modes `main` turns on after `ratatui::init`.
fn leave_modes() {
    let _ = execute!(
        stdout(),
        Print("\x1b[?1006l\x1b[?1000l"),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste
    );
}
