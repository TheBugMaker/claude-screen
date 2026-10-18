//! The Unix socket where `ccs hook` delivers the reports of the ccs-bridge mods.

use std::{
    io,
    path::{Path, PathBuf},
};

use ccs_claude::bridge::Bridge;
use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::mpsc,
};

/// A line `ccs hook` writes.
#[derive(Deserialize)]
struct Report {
    session: u64,
    event: Bridge,
}

/// The socket file, removed when this is dropped.
pub struct Socket {
    path: PathBuf,
}

impl Socket {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Listens on `path` and sends each report, with the key of its session, to `reports`.
pub fn listen(path: PathBuf, reports: mpsc::UnboundedSender<(u64, Bridge)>) -> io::Result<Socket> {
    // Left by a ccs that crashed with the same pid.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    tracing::info!(path = %path.display(), "listening for bridge reports");
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(read(stream, reports.clone()));
                }
                Err(err) => {
                    tracing::warn!(%err, "accepting a bridge connection failed");
                    return;
                }
            }
        }
    });
    Ok(Socket { path })
}

async fn read(stream: UnixStream, reports: mpsc::UnboundedSender<(u64, Bridge)>) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match serde_json::from_str::<Report>(&line) {
            Ok(report) => {
                tracing::debug!(session = report.session, event = ?report.event, "bridge");
                let _ = reports.send((report.session, report.event));
            }
            Err(err) => tracing::warn!(%err, line, "unexpected bridge report"),
        }
    }
}
