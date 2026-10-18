//! A command running in a pseudo-terminal, with its screen kept in a `vt100` parser.
//!
//! A reader thread feeds the program's output to the parser and tells the app to redraw;
//! a waiter thread reports when the program exits. Both are plain threads, because
//! portable-pty's reads and waits block.

use std::{
    io::{ErrorKind, Read, Write},
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc,
    },
    time::Duration,
};

use crossterm::event::{KeyEvent, KeyModifiers, MouseEventKind};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::{Frame, buffer::Buffer, layout::Rect, widgets::Widget};
use tokio::sync::mpsc;
use tui_term::widget::{Cursor, PseudoTerminal};

use crate::{Error, keys};

/// Lines kept above the screen.
const SCROLLBACK: usize = 5000;
/// What programs in tabs are told the terminal is; close to what vt100 emulates.
const TERM: &str = "xterm-256color";
/// How long the waiter lets the reader drain the last output after the program exits.
/// Background processes can keep the terminal open forever.
const DRAIN: Duration = Duration::from_millis(500);

/// Something that happened in a tab's program.
#[derive(Debug)]
pub enum PtyEvent {
    /// The screen changed. Sent once until the next [`Pty::render`].
    Output,
    /// The program exited, with a status such as `exit 0`. Always the last event.
    Exited(String),
}

/// Sender for events from all terminals, tagged with the terminal's key.
pub type EventSender = mpsc::UnboundedSender<(u64, PtyEvent)>;

/// A running (or finished) program in a pseudo-terminal. Dropping it hangs the program up.
pub struct Pty {
    parser: Arc<Mutex<vt100::Parser>>,
    /// Output arrived since the last render.
    dirty: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    master: Box<dyn MasterPty + Send>,
    /// Kept for the whole life of the tab: dropping it sends the program an end of file.
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl Pty {
    /// Runs `command` with `sh -c` in `cwd`, with `env` added, on a terminal of `size` (rows,
    /// columns).
    pub fn spawn(
        command: &str,
        cwd: &Path,
        env: &[(&str, &str)],
        size: (u16, u16),
        key: u64,
        events: EventSender,
    ) -> Result<Self, Error> {
        let (rows, cols) = size;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                ..PtySize::default()
            })
            .map_err(|err| Error::Pty(format!("{err:#}")))?;
        let mut builder = CommandBuilder::new("sh");
        builder.args(["-c", command]);
        // Without a cwd, portable-pty starts the program in $HOME.
        builder.cwd(cwd);
        builder.env("TERM", TERM);
        for (name, value) in env {
            builder.env(name, value);
        }
        let mut child = pair
            .slave
            .spawn_command(builder)
            .map_err(|err| Error::Spawn(format!("{err:#}")))?;
        // Our copy of the slave would keep the terminal open after the program exits.
        drop(pair.slave);
        tracing::info!(key, command, cwd = %cwd.display(), pid = child.process_id(), "tab started");

        let master = pair.master;
        let mut reader = master
            .try_clone_reader()
            .map_err(|err| Error::Pty(format!("{err:#}")))?;
        let writer = master
            .take_writer()
            .map_err(|err| Error::Pty(format!("{err:#}")))?;
        let killer = child.clone_killer();

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
        let dirty = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));

        let (drained, wait_drained) = std_mpsc::channel::<()>();
        std::thread::spawn({
            let parser = parser.clone();
            let dirty = dirty.clone();
            let events = events.clone();
            move || {
                // Dropped when the reader stops, which is what the waiter waits for.
                let _drained = drained;
                let mut buf = [0; 8192];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) => return,
                        Ok(n) => n,
                        Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                        Err(err) => {
                            tracing::debug!(key, %err, "reading the terminal failed");
                            return;
                        }
                    };
                    lock(&parser).process(&buf[..n]);
                    if !dirty.swap(true, Ordering::AcqRel)
                        && events.send((key, PtyEvent::Output)).is_err()
                    {
                        return;
                    }
                }
            }
        });
        std::thread::spawn({
            let exited = exited.clone();
            move || {
                let status = match child.wait() {
                    Ok(status) => match status.signal() {
                        Some(signal) => signal.to_owned(),
                        None => format!("exit {}", status.exit_code()),
                    },
                    Err(err) => format!("wait failed: {err}"),
                };
                exited.store(true, Ordering::Release);
                let _ = wait_drained.recv_timeout(DRAIN);
                tracing::info!(key, status, "tab exited");
                let _ = events.send((key, PtyEvent::Exited(status)));
            }
        });

        Ok(Self {
            parser,
            dirty,
            exited,
            master,
            writer,
            killer,
        })
    }

    /// Sends a key press to the program.
    pub fn key(&mut self, key: KeyEvent) {
        let application_cursor = lock(&self.parser).screen().application_cursor();
        self.write(&keys::encode(key, application_cursor));
    }

    /// Sends pasted text, marked as a paste if the program asked for that.
    pub fn paste(&mut self, text: &str) {
        if lock(&self.parser).screen().bracketed_paste() {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    /// Sends a mouse event at `position` (column, row from 0) of the screen, if the program asked
    /// for SGR mouse reports. `false` when it didn't, and the event is the caller's.
    pub fn mouse(
        &mut self,
        kind: MouseEventKind,
        modifiers: KeyModifiers,
        position: (u16, u16),
    ) -> bool {
        let mode = {
            let parser = lock(&self.parser);
            let screen = parser.screen();
            if screen.mouse_protocol_encoding() != vt100::MouseProtocolEncoding::Sgr {
                return false;
            }
            screen.mouse_protocol_mode()
        };
        if mode == vt100::MouseProtocolMode::None {
            return false;
        }
        self.write(&keys::mouse(kind, modifiers, position, mode));
        true
    }

    /// Changes the terminal size (rows, columns); the program gets SIGWINCH.
    pub fn resize(&mut self, (rows, cols): (u16, u16)) {
        let mut parser = lock(&self.parser);
        if parser.screen().size() == (rows, cols) {
            return;
        }
        parser.screen_mut().set_size(rows, cols);
        if let Err(err) = self.master.resize(PtySize {
            rows,
            cols,
            ..PtySize::default()
        }) {
            tracing::warn!(%err, "resizing the terminal failed");
        }
    }

    /// Scrolls back through the lines above the screen. Programs on the alternate screen,
    /// like `less`, have none.
    pub fn scroll_up(&mut self, lines: usize) {
        let mut parser = lock(&self.parser);
        let screen = parser.screen_mut();
        screen.set_scrollback(screen.scrollback().saturating_add(lines));
    }

    /// Scrolls forward, towards the live screen.
    pub fn scroll_down(&mut self, lines: usize) {
        let mut parser = lock(&self.parser);
        let screen = parser.screen_mut();
        screen.set_scrollback(screen.scrollback().saturating_sub(lines));
    }

    /// The text on screen.
    #[must_use]
    pub fn contents(&self) -> String {
        lock(&self.parser).screen().contents()
    }

    /// Draws the screen into `area` of `buffer`, without the cursor.
    pub fn draw(&self, buffer: &mut Buffer, area: Rect) {
        self.dirty.store(false, Ordering::Release);
        let parser = lock(&self.parser);
        // The real cursor is placed by `render`; tui-term's would be a second one.
        PseudoTerminal::new(parser.screen())
            .cursor(Cursor::default().visibility(false))
            .render(area, buffer);
    }

    /// Draws the screen into `area`, with the cursor if `focused`.
    pub fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        self.draw(frame.buffer_mut(), area);
        let parser = lock(&self.parser);
        let screen = parser.screen();
        if focused && !screen.hide_cursor() && screen.scrollback() == 0 {
            let (row, col) = screen.cursor_position();
            if row < area.height && col < area.width {
                frame.set_cursor_position((area.x + col, area.y + row));
            }
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        if let Err(err) = self
            .writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
        {
            // The program is gone; `Exited` reports that.
            tracing::debug!(%err, "writing to the terminal failed");
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // Its pid may belong to another process by now.
        if !self.exited.load(Ordering::Acquire) {
            let _ = self.killer.kill();
        }
    }
}

/// The parser, even if a thread panicked while holding it: the screen is still drawable.
fn lock(parser: &Mutex<vt100::Parser>) -> MutexGuard<'_, vt100::Parser> {
    parser.lock().unwrap_or_else(PoisonError::into_inner)
}
