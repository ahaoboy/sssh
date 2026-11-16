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

use crate::session::{Msg, SinkNotify};

/// Drives a single SSH channel using the native `Channel` API (not
/// `ChannelStream`), which preserves the ability to send window-change
/// (PTY resize) messages.
///
/// Returns when the channel is closed (EOF/error) or the control channel is
/// dropped.
pub(crate) async fn io_loop(
    mut channel: Channel<client::Msg>,
    mut control_rx: tokio::sync::mpsc::UnboundedReceiver<Msg>,
    notify: Arc<SinkNotify>,
) {
    info!("SSH I/O loop started");

    loop {
        tokio::select! {
            // ── Read SSH channel events ───────────────────────
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => notify.data(lossy(data.to_vec())),
                    // Extended data (stderr) is delivered as regular data:
                    // an interactive terminal shows both interleaved.
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        notify.data(lossy(data.to_vec()))
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        info!("SSH channel closed");
                        notify.close();
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
                            notify.close();
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

fn lossy(data: Vec<u8>) -> String {
    String::from_utf8(data).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}
