//! Background SSH I/O task.
//!
//! Runs in a tokio task: reads from the SSH channel and delivers data to the
//! [`EventSink`]; receives [`Msg`]s to write data, resize the terminal, or
//! disconnect.

use russh::Channel;
use russh::ChannelMsg;
use russh::client;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::session::{EventSink, Msg};

/// Drives a single SSH channel using the native `Channel` API (not
/// `ChannelStream`), which preserves the ability to send window-change
/// (PTY resize) messages.
///
/// Returns when the channel is closed (EOF/error) or the control channel is
/// dropped.
pub(crate) async fn io_loop(
    mut channel: Channel<client::Msg>,
    mut control_rx: tokio::sync::mpsc::UnboundedReceiver<Msg>,
    sink: Arc<dyn EventSink>,
) {
    info!("SSH I/O loop started");

    loop {
        tokio::select! {
            // ── Read SSH channel events ───────────────────────
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        // `data` is `russh::keys::ssh_encoding::Bytes` — convert.
                        let text = match String::from_utf8(data.to_vec()) {
                            Ok(s) => s,
                            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
                        };
                        sink.on_data(text);
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        // Extended data (stderr) — deliver as regular data
                        // since we're in interactive mode.
                        let text = match String::from_utf8(data.to_vec()) {
                            Ok(s) => s,
                            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
                        };
                        sink.on_data(text);
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        info!("SSH channel closed (EOF)");
                        sink.on_data(
                            "\r\n\x1b[33mConnection closed by remote host.\x1b[0m\r\n"
                                .to_string(),
                        );
                        sink.on_closed();
                        break;
                    }
                    _ => {
                        // Ignore other channel messages (exit status, etc.)
                    }
                }
            }

            // ── Control messages (write / resize / disconnect) ─
            msg = control_rx.recv() => {
                match msg {
                    Some(Msg::Data(data)) => {
                        if let Err(e) = channel.data(&data[..]).await {
                            error!(%e, "Write error in SSH I/O loop");
                            sink.on_data(
                                format!("\r\n\x1b[31mWrite error: {e}\x1b[0m\r\n"),
                            );
                            break;
                        }
                    }
                    Some(Msg::Resize { cols, rows }) => {
                        debug!("Resizing PTY to {cols}x{rows}");
                        if let Err(e) = channel
                            .window_change(cols, rows, 0, 0)
                            .await
                        {
                            warn!(%e, "Failed to resize PTY window");
                        }
                    }
                    Some(Msg::Disconnect) => {
                        debug!("Disconnect requested, shutting down I/O loop");
                        break;
                    }
                    None => {
                        debug!("Control channel closed, shutting down I/O loop");
                        break;
                    }
                }
            }
        }
    }
    info!("SSH I/O loop ended");
}
