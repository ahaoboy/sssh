//! Authenticated SSH client: a reusable transport for command execution.
//!
//! [`Session`](crate::Session) is built around an interactive terminal (one
//! channel, one sink). [`Client`] is the non-interactive counterpart: it
//! authenticates once and can then run any number of commands over concurrent
//! `exec` channels. The filesystem API in [`crate::fs`] is layered on top of
//! it.
//!
//! ```no_run
//! # async fn run() -> Result<(), sssh::Error> {
//! use sssh::{Auth, Client, Config};
//!
//! let client = Client::connect(&Config {
//!     host: "example.com".into(),
//!     port: 22,
//!     username: "user".into(),
//!     password: Some("pass".into()),
//!     private_key: None,
//!     command: None,
//!     cols: 80,
//!     rows: 24,
//!     timeout_secs: Some(10),
//!     host_key_verifier: None,
//! })
//! .await?;
//!
//! let out = client.exec("uname -a").await?;
//! println!("{}", out.stdout_lossy());
//! # Ok(()) }
//! ```

use russh::client;
use russh::{ChannelMsg, Disconnect};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::debug;

use crate::error::Error;
use crate::handler::ClientHandler;
use crate::output::Output;
use crate::session::{Auth, Config, authenticate, parse_target};
use crate::shell::shell_quote;

struct ClientInner {
    handle: client::Handle<ClientHandler>,
    /// Remote `$HOME`, used to expand `~` and relative paths.
    home: OnceLock<String>,
    /// `user@host` label used in error messages.
    label: String,
}

/// How much data a [`Client::stream`] call moved and whether it completed.
pub(crate) struct Streamed {
    pub output: Output,
    /// Bytes successfully handed to the remote command's stdin.
    pub sent: u64,
    /// Bytes received from the remote command's stdout.
    pub received: u64,
    /// Whether the local input reached EOF (i.e. nothing was cut short).
    pub finished: bool,
}

/// An authenticated, non-interactive SSH connection.
///
/// Cloning a `Client` is cheap (an `Arc` bump); every clone shares the same
/// transport and can run commands concurrently. Drop the last clone to close
/// the connection.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("label", &self.label())
            .field("home", &self.home())
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connect and authenticate using [`Config`].
    ///
    /// Only the connection fields are used; `command`, `cols` and `rows` are
    /// ignored. Once connected, the remote `$HOME` is probed so relative
    /// paths and `~` can be expanded for [`crate::fs`].
    pub async fn connect(config: &Config) -> Result<Self, Error> {
        let handle = authenticate(config).await?;
        let client = Self {
            inner: Arc::new(ClientInner {
                handle,
                home: OnceLock::new(),
                label: format!("{}@{}", config.username, config.host),
            }),
        };

        let home = match client.exec_sh(r#"printf '%s' "$HOME""#).await {
            Ok(out) => out.stdout_lossy().trim().to_string(),
            Err(e) => {
                debug!("remote $HOME probe failed: {e}");
                String::new()
            }
        };
        if home.is_empty() {
            debug!(target: "sssh", "remote $HOME probe returned nothing");
        }
        let _ = client.inner.home.set(home);
        Ok(client)
    }

    /// Connect to `[user@]host` with the given credentials and timeout.
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), sssh::Error> {
    /// let client = sssh::Client::connect_target(
    ///     "user@host",
    ///     22,
    ///     sssh::Auth::password("pass"),
    ///     Some(10),
    /// )
    /// .await?;
    /// # Ok(()) }
    /// ```
    pub async fn connect_target(
        target: &str,
        port: u16,
        auth: Auth,
        timeout_secs: Option<u64>,
    ) -> Result<Self, Error> {
        let (username, host) = parse_target(target).map_err(Error::Other)?;
        let (password, private_key) = auth.into();
        Self::connect(&Config {
            host,
            port,
            username,
            password,
            private_key,
            command: None,
            cols: 80,
            rows: 24,
            timeout_secs,
            host_key_verifier: None,
        })
        .await
    }

    /// Remote `$HOME` (empty when the probe failed).
    pub fn home(&self) -> &str {
        self.inner.home.get().map(String::as_str).unwrap_or("")
    }

    /// Connection label (`user@host`).
    pub fn label(&self) -> &str {
        &self.inner.label
    }

    /// A filesystem handle bound to this connection.
    pub fn fs(&self) -> crate::fs::Fs {
        crate::fs::Fs::new(self.clone())
    }

    /// Run `command` and capture its output.
    pub async fn exec(&self, command: &str) -> Result<Output, Error> {
        self.exec_stream(command, tokio::io::sink(), tokio::io::sink())
            .await
    }

    /// Run a POSIX script through `sh -c`, regardless of the login shell.
    ///
    /// The remote login shell may be fish or zsh, which reject POSIX
    /// constructs such as `d=$(…)`. Wrapping in `sh -c` keeps behaviour
    /// identical on every host.
    pub async fn exec_sh(&self, script: &str) -> Result<Output, Error> {
        self.exec(&format!("sh -c {}", shell_quote(script))).await
    }

    /// Run `command`, streaming `input` to its stdin.
    ///
    /// Writing runs concurrently with the read loop: writing first and
    /// reading afterwards deadlocks whenever the command exits early (disk
    /// full, `cat: write error`) because the SSH window never reopens.
    pub async fn exec_with_input(
        &self,
        command: &str,
        input: impl AsRef<[u8]>,
    ) -> Result<Output, Error> {
        let input = std::io::Cursor::new(input.as_ref().to_vec());
        self.exec_stream_with_input(command, input, tokio::io::sink(), tokio::io::sink())
            .await
    }

    /// Run `command`, streaming its stdout/stderr to the given writers as
    /// data arrives, and return the captured output.
    ///
    /// ```no_run
    /// # async fn run(client: sssh::Client) -> Result<(), sssh::Error> {
    /// let out = client
    ///     .exec_stream("cat /etc/hostname", tokio::io::stdout(), tokio::io::stderr())
    ///     .await?;
    /// println!("exit: {:?}", out.code());
    /// # Ok(()) }
    /// ```
    pub async fn exec_stream<O, E>(&self, command: &str, out: O, err: E) -> Result<Output, Error>
    where
        O: AsyncWrite + Unpin + Send,
        E: AsyncWrite + Unpin + Send,
    {
        Ok(self
            .stream(command, tokio::io::empty(), out, err, true)
            .await?
            .output)
    }

    /// Like [`Client::exec_stream`], with `input` streamed to stdin.
    ///
    /// `input` is moved into the background writer, hence `'static`;
    /// `tokio::fs::File` and in-memory cursors satisfy this.
    pub async fn exec_stream_with_input<I, O, E>(
        &self,
        command: &str,
        input: I,
        out: O,
        err: E,
    ) -> Result<Output, Error>
    where
        I: AsyncRead + Unpin + Send + 'static,
        O: AsyncWrite + Unpin + Send,
        E: AsyncWrite + Unpin + Send,
    {
        Ok(self.stream(command, input, out, err, true).await?.output)
    }

    /// Run `command` and fail with the remote stderr when it exits non-zero.
    pub async fn exec_checked(&self, command: &str) -> Result<Output, Error> {
        let out = self.exec(command).await?;
        if !out.success() {
            return Err(Error::Other(format!(
                "`{command}` failed on {} ({}): {}",
                self.label(),
                out.status,
                out.stderr_lossy().trim(),
            )));
        }
        Ok(out)
    }

    /// Expand `~` and relative paths against the remote home directory.
    pub fn expand_path(&self, path: &str) -> PathBuf {
        PathBuf::from(crate::fs::expand_with_home(path, self.home()))
    }

    /// Add `dir` to `PATH` by appending one line to `shell`'s config file.
    ///
    /// The edit runs through a POSIX shell (`sh -c`), so it works whatever the
    /// login shell is, and it is idempotent: the line is only appended when the
    /// file does not already contain it.
    ///
    /// ```no_run
    /// # async fn run(client: sssh::Client) -> Result<(), sssh::Error> {
    /// client
    ///     .add_to_path(sssh::Shell::Bash, "~/.local/bin")
    ///     .await?;
    /// # Ok(()) }
    /// ```
    /// ```no_run
    /// # async fn run(client: sssh::Client) -> Result<(), sssh::Error> {
    /// let file = client
    ///     .add_to_path(sssh::Shell::Bash, "~/.local/bin")
    ///     .await?;
    /// println!("added to {}", file.display());
    /// # Ok(()) }
    /// ```
    pub async fn add_to_path(
        &self,
        shell: which_shell::Shell,
        dir: &str,
    ) -> Result<std::path::PathBuf, Error> {
        let config = shell.path_config().ok_or_else(|| {
            Error::Other(format!(
                "{shell} has no configuration file to add '{dir}' to"
            ))
        })?;
        let line = shell.path_line(dir).expect("path_config implies path_line");

        // `exec_sh`: the script is POSIX, but the login shell may be fish or
        // zsh, which reject `rc=…` and friends.
        let script = crate::path::add_line_command(config.file, &line);
        let out = self.exec_sh(&script).await?;
        if !out.success() {
            return Err(Error::Other(format!(
                "adding '{dir}' to PATH failed on {} ({}): {}",
                self.label(),
                out.status,
                out.stderr_lossy().trim()
            )));
        }

        // Report the file in the remote home, which is what was written.
        Ok(std::path::Path::new(self.home()).join(config.file))
    }

    /// Send a graceful SSH disconnect.
    pub async fn disconnect(&self) {
        let _ = self
            .inner
            .handle
            .disconnect(Disconnect::ByApplication, "session done", "en")
            .await;
    }

    /// Shared engine behind every `exec*` method.
    ///
    /// `capture` controls whether stdout/stderr are also buffered into the
    /// returned [`Output`]; pass `false` when streaming a large payload to a
    /// writer so it is never held in memory.
    pub(crate) async fn stream<I, O, E>(
        &self,
        command: &str,
        input: I,
        mut out: O,
        mut err: E,
        capture: bool,
    ) -> Result<Streamed, Error>
    where
        I: AsyncRead + Unpin + Send + 'static,
        O: AsyncWrite + Unpin + Send,
        E: AsyncWrite + Unpin + Send,
    {
        let channel = self
            .inner
            .handle
            .channel_open_session()
            .await
            .map_err(|e| Error::Other(format!("failed to open SSH channel: {e}")))?;
        channel
            .exec(true, command.as_bytes())
            .await
            .map_err(|e| Error::Other(format!("failed to execute remote command: {e}")))?;

        let (mut rx, tx) = channel.split();
        let sent = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));

        let writer = {
            let sent = Arc::clone(&sent);
            let finished = Arc::clone(&finished);
            tokio::spawn(async move {
                let mut src = input;
                let mut buf = vec![0u8; 32 * 1024];
                loop {
                    match src.read(&mut buf).await {
                        Ok(0) => {
                            finished.store(true, Ordering::Relaxed);
                            break;
                        }
                        Ok(n) => {
                            if tx.data_bytes(buf[..n].to_vec()).await.is_err() {
                                return;
                            }
                            sent.fetch_add(n as u64, Ordering::Relaxed);
                        }
                        Err(_) => return,
                    }
                }
                // Always signal EOF so commands that read stdin terminate.
                let _ = tx.eof().await;
            })
        };

        let mut output = Output::default();
        let mut received = 0u64;
        loop {
            match rx.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    received += data.len() as u64;
                    let _ = out.write_all(&data).await;
                    if capture {
                        output.stdout.extend_from_slice(&data);
                    }
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    // stderr is always captured: it is small and carries the
                    // error message callers report on failure.
                    let _ = err.write_all(&data).await;
                    output.stderr.extend_from_slice(&data);
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    output.status = crate::output::ExitStatus::from_code(exit_status);
                }
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    output.status =
                        crate::output::ExitStatus::from_signal(format!("{signal_name:?}"));
                }
                // `Eof` only means no more stdout/stderr: the server still
                // sends `ExitStatus` and `Close` afterwards. Breaking here
                // would drop the exit status.
                Some(ChannelMsg::Eof) => {}
                Some(ChannelMsg::Close) | None => break,
                _ => {}
            }
        }

        // The command may have exited without draining stdin; stop the writer
        // instead of waiting for window space that will never be granted.
        writer.abort();
        let _ = out.flush().await;
        let _ = err.flush().await;

        Ok(Streamed {
            output,
            sent: sent.load(Ordering::Relaxed),
            received,
            finished: finished.load(Ordering::Relaxed),
        })
    }
}
