//! Terminal-integration helpers for the `sssh` binary: an interactive
//! shell driver and an idle-session driver, each with raw-mode, stdin
//! forwarding, and Ctrl-C handling.
//!
//! For programmatic use, prefer the generic [`crate::Session`] API directly:
//! - [`Session::exec`] — run a command and get its [`Output`](crate::Output)
//!   back (no terminal side effects).
//! - [`Session::exec_stream`] — run a command, streaming to any
//!   [`AsyncWrite`] writers, and also return [`Output`](crate::Output).
//! - [`Session::connect`] / [`Session::connect_idle`] — open a session with
//!   a custom [`EventSink`](crate::EventSink) and drive it yourself.

use crate::{Config, Error, EventSink, Session};
use std::io::{IsTerminal, Write};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Minimal [`EventSink`]: remote output → stdout, disconnect → channel.
struct StdoutSink {
    closed_tx: mpsc::UnboundedSender<()>,
}

impl EventSink for StdoutSink {
    fn on_connected(&self) {}

    fn on_data(&self, data: String) {
        let mut out = std::io::stdout();
        let _ = out.write_all(data.as_bytes());
        let _ = out.flush();
    }

    fn on_closed(&self) {
        let _ = self.closed_tx.send(());
    }
}

/// RAII guard that restores the terminal on drop so we never leave it in
/// raw mode.
struct RawGuard;

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// **Shell mode** — connect, open an interactive PTY shell, forward raw
/// stdin bytes verbatim, and disconnect on exit / Ctrl-C.
///
/// Raw stdin bytes are forwarded verbatim (including terminal query
/// responses like DA1/OSC 11) so the remote shell receives them intact,
/// matching OpenSSH behavior. In raw interactive mode, `Ctrl+C` (0x03)
/// exits the client.
pub async fn run_shell(config: Config) -> Result<(), Error> {
    let (closed_tx, mut closed_rx) = mpsc::unbounded_channel();
    let sink = StdoutSink { closed_tx };

    let mut session = Session::connect(config, sink).await?;

    let interactive = std::io::stdin().is_terminal();
    let _guard = if interactive {
        crossterm::terminal::enable_raw_mode()?;
        Some(RawGuard)
    } else {
        None
    };

    // Read stdin directly in the select loop — no extra spawned task.
    use tokio::io::AsyncReadExt;
    let mut stdin = tokio::io::stdin();
    let mut stdin_buf = vec![0u8; 4096];
    let mut closed = false;

    loop {
        tokio::select! {
            // Read raw stdin bytes and forward to the remote.
            result = stdin.read(&mut stdin_buf) => {
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // In raw interactive mode, Ctrl+C (0x03) exits
                        // (raw mode prevents it from reaching SIGINT).
                        if interactive && n == 1 && stdin_buf[0] == 0x03 {
                            info!("Ctrl-C received, exiting");
                            break;
                        }
                        if let Err(e) = session.write(stdin_buf[..n].to_vec()) {
                            warn!("Write error: {e}");
                            break;
                        }
                    }
                }
            }
            _ = closed_rx.recv() => {
                closed = true;
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }

    if closed {
        println!("\r\n\x1b[33mConnection closed by remote host.\x1b[0m\r\n");
    }
    session.disconnect();
    Ok(())
}

/// **Idle mode** — authenticate and keep the connection alive until
/// interrupted (Ctrl-C or remote disconnect).
pub async fn run_idle(config: Config) -> Result<(), Error> {
    let (closed_tx, mut closed_rx) = mpsc::unbounded_channel();
    let sink = StdoutSink { closed_tx };

    let mut session = Session::connect_idle(config, sink).await?;

    info!("Idle session established; waiting for Ctrl-C or disconnect");
    tokio::select! {
        _ = closed_rx.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }

    session.disconnect();
    Ok(())
}

/// **Cmd mode** — execute a single command non-interactively, stream its
/// stdout/stderr live to the provided writers, and exit with the remote
/// process's exit code.
///
/// `out` and `err` are the stdout/stderr sinks — pass
/// [`tokio::io::stdout`] / [`tokio::io::stderr`] for terminal output, or
/// any [`AsyncWrite`](tokio::io::AsyncWrite) for redirection.
///
/// Ctrl-C is handled gracefully: the connection is interrupted and the
/// process returns `Ok(())` (no error).  If the command completes normally,
/// the process exits with the remote exit status.
///
/// This is the function behind `sssh -m cmd -c <CMD> <target>`.
pub async fn run_cmd<W1, W2>(config: Config, command: &str, out: W1, err: W2) -> Result<(), Error>
where
    W1: tokio::io::AsyncWrite + Unpin + Send,
    W2: tokio::io::AsyncWrite + Unpin + Send,
{
    let result = tokio::select! {
        res = Session::exec_stream(config, command, out, err) => res?,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nInterrupted");
            return Ok(());
        }
    };
    if let Some(code) = result.code() {
        std::process::exit(code);
    }
    Ok(())
}
