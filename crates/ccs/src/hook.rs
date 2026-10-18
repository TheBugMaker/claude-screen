//! `ccs hook`: how the ccs-bridge mod inside an interactive `claude` reports to its ccs.

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
};

use serde_json::{Value, json};

/// Sends the JSON report on stdin, tagged with `session`, as one line to `socket`. Quietly does
/// nothing when it can't: Claude must never be held up or shown an error by it.
pub fn run(socket: &Path, session: u64) {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(event) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    let line = format!("{}\n", json!({ "session": session, "event": event }));
    if let Ok(mut stream) = UnixStream::connect(socket) {
        let _ = stream.write_all(line.as_bytes());
    }
}
