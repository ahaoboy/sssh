//! The reusable SSH session abstraction.
//!
//! [`Session`] encapsulates the full lifecycle of an interactive SSH
//! connection: TCP connect → authenticate → open PTY shell (or run a command)
//! → background read/write loop. Consumers receive data and lifecycle events
//! through the [`EventSink`] trait, keeping the module free of any UI or
//! host-framework dependencies.
//!
//! For non-interactive use-cases see [`Session::connect_idle`] (authenticate
//! and keep the session alive) and [`Session::exec`] (run a single command and
//! return its captured output + exit status).

use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, decode_secret_key};
use russh::{ChannelMsg, Disconnect};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::error::Error;
use crate::handler::{AcceptAll, ClientHandler, HostKeyVerifier};
use crate::io_loop::io_loop;

// ── Public types ────────────────────────────────────────────────────────

/// Connection / authentication configuration.
#[derive(serde::Deserialize)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    /// Optional remote command to execute instead of the default shell.
    ///
    /// Used by [`Session::connect`]. Ignored by [`Session::connect_idle`] and
    /// [`Session::exec`] (the latter takes its command explicitly).
    pub command: Option<String>,
    pub cols: u32,
    pub rows: u32,
    /// Connection timeout in seconds. `None` means no timeout.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Host-key verification policy. Defaults to [`AcceptAll`] (insecure).
    #[serde(skip, default = "default_verifier")]
    pub host_key_verifier: Option<Arc<dyn HostKeyVerifier>>,
}

fn default_verifier() -> Option<Arc<dyn HostKeyVerifier>> {
    None
}

// Manual impls to skip the trait-object field in Debug output.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("private_key", &self.private_key.as_ref().map(|_| "***"))
            .field("command", &self.command)
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("timeout_secs", &self.timeout_secs)
            .field(
                "host_key_verifier",
                &self.host_key_verifier.as_ref().map(|_| "…"),
            )
            .finish()
    }
}

impl Clone for Config {
    fn clone(&self) -> Self {
        Self {
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            password: self.password.clone(),
            private_key: self.private_key.clone(),
            command: self.command.clone(),
            cols: self.cols,
            rows: self.rows,
            timeout_secs: self.timeout_secs,
            host_key_verifier: self.host_key_verifier.clone(),
        }
    }
}

/// Messages sent from the session API into the background I/O task.
#[derive(Debug)]
pub(crate) enum Msg {
    /// Write terminal input to the SSH channel.
    Data(Vec<u8>),
    /// Resize the remote PTY.
    Resize { cols: u32, rows: u32 },
    /// Request the I/O loop to shut down gracefully.
    Disconnect,
}

/// Sink for events delivered by the SSH session.
///
/// Implement this trait to bridge session events into your UI or event system.
/// All methods are synchronous — the session calls them from the async runtime,
/// so implementations should be non-blocking (e.g. post to a channel or emit
/// an event).
pub trait EventSink: Send + Sync + 'static {
    /// The connection is fully established and the session is ready for I/O.
    fn on_connected(&self);
    /// A chunk of output was received from the remote host.
    fn on_data(&self, data: String);
    /// The connection was closed (EOF, error, or explicit disconnect).
    fn on_closed(&self);
}

/// Captured output of a one-shot command ([`Session::exec`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use = "Output contains the exit code — check it"]
pub struct Output {
    /// Captured stdout of the command.
    pub stdout: String,
    /// Captured stderr of the command.
    pub stderr: String,
    /// Exit status reported by the remote host, when available.
    pub exit_code: Option<u32>,
}

/// Authentication credentials for a session.
///
/// Used by [`exec`] and [`Config`]. Pass [`Auth::key`] for private-key auth
/// or [`Auth::password`] for password auth.
#[derive(Debug, Clone)]
pub enum Auth {
    Password(String),
    PrivateKey(String),
}

impl Auth {
    /// Authenticate with a password.
    pub fn password(p: impl Into<String>) -> Self {
        Self::Password(p.into())
    }

    /// Authenticate with a PEM/OpenSSH private key string.
    pub fn key(k: impl Into<String>) -> Self {
        Self::PrivateKey(k.into())
    }
}

impl From<Auth> for (Option<String>, Option<String>) {
    fn from(auth: Auth) -> Self {
        match auth {
            Auth::Password(p) => (Some(p), None),
            Auth::PrivateKey(k) => (None, Some(k)),
        }
    }
}

/// Parse `[user@]host` into `(username, host)`.
pub fn parse_target(target: &str) -> Result<(String, String), String> {
    match target.split_once('@') {
        Some((user, host)) if !user.is_empty() && !host.is_empty() => {
            Ok((user.to_string(), host.to_string()))
        }
        _ => Err(format!("invalid target '{target}', expected user@host")),
    }
}

/// Connect to `user@host:port`, run `command`, and return its captured
/// output + exit status.
///
/// This is a convenience wrapper around [`Session::exec`] for the common
/// "run one command and get the result" case — no terminal side effects,
/// no `Config` boilerplate. For streaming output or interactive shells,
/// use [`Session::exec_stream`] / [`Session::connect`] directly.
///
/// ```no_run
/// # async fn run() -> Result<(), sssh::Error> {
/// let out = sssh::exec("user@host", 22, sssh::Auth::password("pass"), "uname -a").await?;
/// println!("{}", out.stdout);
/// println!("exit: {:?}", out.exit_code);
/// # Ok(()) }
/// ```
pub async fn exec(target: &str, port: u16, auth: Auth, command: &str) -> Result<Output, Error> {
    let (username, host) = parse_target(target).map_err(Error::Other)?;
    let (password, private_key) = auth.into();
    Session::exec_stream(
        Config {
            host,
            port,
            username,
            password,
            private_key,
            command: None,
            cols: 80,
            rows: 24,
            timeout_secs: None,
            host_key_verifier: None,
        },
        command,
        tokio::io::sink(),
        tokio::io::sink(),
    )
    .await
}

// ── Session ─────────────────────────────────────────────────────────────

/// An active SSH session.
///
/// Owns the russh client handle and (for interactive sessions) the background
/// read/write task. Dropping the session aborts the task and closes the
/// connection.
pub struct Session {
    handle: Option<client::Handle<ClientHandler>>,
    control_tx: Option<mpsc::UnboundedSender<Msg>>,
    reader_task: Option<JoinHandle<()>>,
    sink: Option<Arc<dyn EventSink>>,
}

impl Session {
    /// Connect, authenticate, and open an interactive PTY shell — or, when
    /// `config.command` is set, execute that command interactively.
    ///
    /// On success [`EventSink::on_connected`] is invoked and the background
    /// I/O task is running.
    pub async fn connect<S: EventSink>(config: Config, sink: S) -> Result<Self, Error> {
        let sink: Arc<dyn EventSink> = Arc::new(sink);

        info!(
            "Connecting to {}@{}:{} (auth: {}, cmd: {}, cols: {}x{})",
            config.username,
            config.host,
            config.port,
            if config.private_key.is_some() {
                "key"
            } else {
                "password"
            },
            config.command.as_deref().unwrap_or("(shell)"),
            config.cols,
            config.rows,
        );

        let handle = authenticate(&config).await?;

        // ── Open channel + PTY + shell ────────────────────────
        debug!("Opening session channel…");
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| Error::Other(format!("Failed to open channel: {e}")))?;

        debug!("Requesting PTY ({}x{})", config.cols, config.rows);
        channel
            .request_pty(false, "xterm-256color", config.cols, config.rows, 0, 0, &[])
            .await
            .map_err(|e| Error::Other(format!("Failed to request PTY: {e}")))?;

        if let Some(ref cmd) = config.command {
            info!("Executing remote command: {cmd}");
            channel
                .exec(true, cmd.as_bytes())
                .await
                .map_err(|e| Error::Other(format!("Failed to execute command: {e}")))?;
        } else {
            debug!("Starting interactive shell");
            channel
                .request_shell(false)
                .await
                .map_err(|e| Error::Other(format!("Failed to start shell: {e}")))?;
        }

        // ── Spawn background I/O task ─────────────────────────
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let reader_task = tokio::spawn(io_loop(channel, control_rx, Arc::clone(&sink)));

        let session = Session {
            handle: Some(handle),
            control_tx: Some(control_tx),
            reader_task: Some(reader_task),
            sink: Some(sink),
        };

        session.sink.as_ref().expect("sink just set").on_connected();
        info!("SSH session ready");
        Ok(session)
    }

    /// Connect and authenticate only — no channel, shell, or I/O loop is
    /// started. The session stays alive (idle) until [`disconnect`][Self::disconnect]
    /// is called or the session is dropped.
    ///
    /// Useful when the caller only needs an authenticated transport (e.g. to
    /// open channels later via the raw russh handle) or wants to keep a
    /// connection warm. [`EventSink::on_connected`] is invoked on success.
    pub async fn connect_idle<S: EventSink>(config: Config, sink: S) -> Result<Self, Error> {
        let sink: Arc<dyn EventSink> = Arc::new(sink);

        info!(
            "Connecting (idle) to {}@{}:{} (auth: {})",
            config.username,
            config.host,
            config.port,
            if config.private_key.is_some() {
                "key"
            } else {
                "password"
            },
        );

        let handle = authenticate(&config).await?;

        let session = Session {
            handle: Some(handle),
            control_tx: None,
            reader_task: None,
            sink: Some(sink),
        };

        session.sink.as_ref().expect("sink just set").on_connected();
        info!("Idle SSH session ready");
        Ok(session)
    }

    /// Connect, authenticate, execute a single command, capture its output and
    /// exit status, then close the channel and disconnect — all in one call.
    ///
    /// No PTY is requested and no [`EventSink`] is required, making this
    /// ideal for scripted/programmatic use (the "cmd" mode). Output is
    /// captured into the returned [`Output`] without streaming.
    pub async fn exec(config: Config, command: &str) -> Result<Output, Error> {
        Self::exec_stream(config, command, tokio::io::sink(), tokio::io::sink()).await
    }

    /// Connect, authenticate, execute a single command, stream its stdout/stderr
    /// to the provided writers as it arrives, and return the captured output +
    /// exit status.
    ///
    /// No PTY is requested. Stdin is closed immediately after `exec` (EOF is
    /// sent) so commands that read stdin don't block waiting for input that
    /// will never come — equivalent to `ssh host command < /dev/null`.
    pub async fn exec_stream<W1, W2>(
        config: Config,
        command: &str,
        mut out: W1,
        mut err: W2,
    ) -> Result<Output, Error>
    where
        W1: tokio::io::AsyncWrite + Unpin + Send,
        W2: tokio::io::AsyncWrite + Unpin + Send,
    {
        info!(
            "Executing one-shot command on {}@{}:{} (auth: {}): {command}",
            config.username,
            config.host,
            config.port,
            if config.private_key.is_some() {
                "key"
            } else {
                "password"
            },
        );

        let handle = authenticate(&config).await?;

        debug!("Opening session channel…");
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| Error::Other(format!("Failed to open channel: {e}")))?;

        debug!("Executing remote command: {command}");
        channel
            .exec(true, command.as_bytes())
            .await
            .map_err(|e| Error::Other(format!("Failed to execute command: {e}")))?;

        // No stdin to provide; signal EOF immediately so commands that read
        // stdin (e.g. an interactive shell started by a misbehaving command)
        // don't block forever waiting for input.
        let _ = channel.eof().await;

        let mut result = Output::default();

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    let _ = out.write_all(&data).await;
                    let _ = out.flush().await;
                    result.stdout.push_str(&String::from_utf8_lossy(&data));
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    // ext == 1 is stderr per RFC 4254; route all extended
                    // data to stderr for simplicity.
                    let _ = err.write_all(&data).await;
                    let _ = err.flush().await;
                    result.stderr.push_str(&String::from_utf8_lossy(&data));
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    result.exit_code = Some(exit_status);
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                    debug!("Command channel closed (EOF/Close)");
                    break;
                }
                _ => {}
            }
        }

        let _ = channel.close().await;
        let _ = handle
            .disconnect(Disconnect::ByApplication, "session done", "en")
            .await;

        info!(
            "Command finished: exit={:?}, stdout={}B, stderr={}B",
            result.exit_code,
            result.stdout.len(),
            result.stderr.len(),
        );
        Ok(result)
    }

    /// Send terminal input to the remote host.
    ///
    /// Accepts anything that can be converted into bytes: `String`, `&str`,
    /// `Vec<u8>`, `&[u8]`, etc.
    pub fn write(&self, data: impl Into<Vec<u8>>) -> Result<(), Error> {
        let tx = self
            .control_tx
            .as_ref()
            .ok_or_else(|| Error::Other("Not connected".into()))?;
        tx.send(Msg::Data(data.into()))
            .map_err(|_| Error::Other("Connection closed".into()))
    }

    /// Resize the remote PTY.
    pub fn resize(&self, cols: u32, rows: u32) -> Result<(), Error> {
        let tx = self
            .control_tx
            .as_ref()
            .ok_or_else(|| Error::Other("Not connected".into()))?;
        tx.send(Msg::Resize { cols, rows })
            .map_err(|_| Error::Other("Connection closed".into()))
    }

    /// Whether the session is still considered active.
    pub fn is_connected(&self) -> bool {
        self.handle.is_some()
    }

    /// Disconnect: signals the background I/O task to stop, sends a graceful
    /// SSH disconnect message to the server, and notifies the sink via
    /// [`EventSink::on_closed`].
    pub fn disconnect(&mut self) {
        info!("Disconnecting SSH session");

        // Signal the I/O loop to shut down first.
        if let Some(tx) = self.control_tx.take() {
            let _ = tx.send(Msg::Disconnect);
        }

        // Send a graceful SSH-level disconnect using the owned handle.
        if let Some(handle) = self.handle.take() {
            tokio::spawn(async move {
                let _ = handle
                    .disconnect(Disconnect::ByApplication, "session done", "en")
                    .await;
            });
        }

        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
        if let Some(sink) = self.sink.take() {
            sink.on_closed();
        }
        debug!("SSH session fully disconnected");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Safety net: if `disconnect` was not called, still abort the task.
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
        self.handle.take();
        self.control_tx.take();
    }
}

// ── Internal helpers ────────────────────────────────────────────────────

/// TCP connect + authenticate, returning the live client handle.
///
/// Shared by [`Session::connect`], [`Session::connect_idle`] and
/// [`Session::exec_stream`].
async fn authenticate(config: &Config) -> Result<client::Handle<ClientHandler>, Error> {
    let client_config = client::Config::default();

    // Resolve the host-key verifier (default: accept all).
    let verifier: Arc<dyn HostKeyVerifier> = config
        .host_key_verifier
        .clone()
        .unwrap_or_else(|| Arc::new(AcceptAll));

    let handler = ClientHandler {
        host: config.host.clone(),
        verifier,
    };

    debug!(
        "Received config: host={}, port={}, user={}, has_key={}, has_pass={}, command={:?}",
        config.host,
        config.port,
        config.username,
        config.private_key.is_some(),
        config.password.is_some(),
        config.command,
    );

    // ── TCP connect (with optional timeout) ──────────────────
    let connect_fut = client::connect(
        Arc::new(client_config),
        (config.host.as_str(), config.port),
        handler,
    );

    let mut handle = if let Some(secs) = config.timeout_secs {
        tokio::time::timeout(Duration::from_secs(secs), connect_fut)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(Error::Connection)?
    } else {
        connect_fut.await.map_err(Error::Connection)?
    };

    info!("TCP connection established, authenticating…");

    // ── Authenticate ──────────────────────────────────────
    let auth_result = if let Some(ref key_str) = config.private_key {
        if key_str.trim().is_empty() {
            warn!("Empty private key provided");
            return Err(Error::Other("Private key is empty".into()));
        }
        debug!("Authenticating with private key…");
        let key_pair = decode_secret_key(key_str, None)
            .map_err(|e| Error::Auth(format!("Invalid private key: {e}")))?;
        // For RSA keys, russh defaults to legacy `ssh-rsa` (SHA-1) when
        // `hash_alg` is `None`, which modern servers reject. Force SHA-256
        // (`rsa-sha2-256`) — the constructor ignores this for non-RSA keys.
        let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key_pair), Some(HashAlg::Sha256));
        handle
            .authenticate_publickey(&config.username, key_with_alg)
            .await
            .map_err(|e| Error::Auth(format!("Authentication failed: {e}")))?
    } else if let Some(ref password) = config.password {
        debug!("Authenticating with password…");
        handle
            .authenticate_password(&config.username, password)
            .await
            .map_err(|e| Error::Auth(format!("Authentication failed: {e}")))?
    } else {
        warn!("No authentication method provided");
        return Err(Error::Other("No authentication method provided".into()));
    };

    if !auth_result.success() {
        warn!("Authentication rejected by server");
        return Err(Error::Auth("Authentication rejected by server".into()));
    }
    info!("Authentication succeeded");

    Ok(handle)
}
