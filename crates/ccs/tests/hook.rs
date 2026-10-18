//! `ccs hook`, as the ccs-bridge mod runs it.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde_json::{Value, json};

/// Runs `ccs hook <socket> 3` with `report` on stdin.
fn hook(socket: &Path, report: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ccs"))
        .args(["hook".as_ref(), socket.as_os_str(), "3".as_ref()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(report.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn socket_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ccs-hook-{name}-{}.sock", std::process::id()))
}

#[test]
fn forwards_the_report_tagged_with_its_session() {
    let path = socket_path("forward");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();

    let output = hook(&path, "{\"ev\":\"ready\",\n\"session_id\":\"5c1c\"}\n");
    let (stream, _) = listener.accept().unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert!(output.status.success());
    // Claude shows a hook's output; this one has none.
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap(),
        json!({ "session": 3, "event": { "ev": "ready", "session_id": "5c1c" } })
    );
}

#[test]
fn says_nothing_when_ccs_is_gone() {
    let output = hook(&socket_path("gone"), "{\"ev\":\"idle\"}");
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}
