//! Runs real programs in a pseudo-terminal.

use std::{path::Path, time::Duration};

use ccs_term::{Pty, PtyEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

const TIMEOUT: Duration = Duration::from_secs(5);

fn spawn(command: &str) -> (Pty, mpsc::UnboundedReceiver<(u64, PtyEvent)>) {
    let (events, received) = mpsc::unbounded_channel();
    let pty = Pty::spawn(
        command,
        Path::new("/"),
        &[("CCS_TEST", "set")],
        (24, 80),
        7,
        events,
    )
    .unwrap();
    (pty, received)
}

/// Waits for the program to exit and returns its status.
async fn exited(events: &mut mpsc::UnboundedReceiver<(u64, PtyEvent)>) -> String {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            match events.recv().await {
                Some((7, PtyEvent::Exited(status))) => return status,
                Some((7, PtyEvent::Output)) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    })
    .await
    .expect("the program did not exit")
}

/// Waits until the screen satisfies `done`.
async fn screen_shows(pty: &Pty, done: impl Fn(&str) -> bool) {
    let waited = tokio::time::timeout(TIMEOUT, async {
        while !done(&pty.contents()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "screen:\n{}", pty.contents());
}

#[tokio::test]
async fn shows_output_and_reports_the_exit() {
    let (pty, mut events) = spawn("printf hello; pwd; exit 3");
    assert_eq!(exited(&mut events).await, "exit 3");
    assert_eq!(pty.contents(), "hello/");
}

#[tokio::test]
async fn adds_the_env_vars() {
    let (pty, mut events) = spawn("printf \"$CCS_TEST\"");
    assert_eq!(exited(&mut events).await, "exit 0");
    assert_eq!(pty.contents(), "set");
}

#[tokio::test]
async fn sends_keys_to_the_program() {
    let (mut pty, mut events) = spawn("read line; echo \"got $line\"");
    for c in "abc".chars() {
        pty.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    pty.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(exited(&mut events).await, "exit 0");
    assert!(pty.contents().contains("got abc"), "{}", pty.contents());
}

#[tokio::test]
async fn resizes_the_terminal() {
    let (mut pty, _events) = spawn("read _; stty size; sleep 5");
    pty.resize((10, 40));
    pty.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    screen_shows(&pty, |screen| screen.contains("10 40")).await;
}

#[tokio::test]
async fn dropping_hangs_up_the_program() {
    let (pty, mut events) = spawn("sleep 30");
    drop(pty);
    let status = exited(&mut events).await;
    assert_ne!(status, "exit 0");
}
