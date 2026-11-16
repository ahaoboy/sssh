//! `sssh` — a reusable SSH session library.
//!
//! Encapsulates the full lifecycle of an SSH connection using [`russh`]:
//! TCP connect → authenticate → open PTY shell (or run a command) →
//! background read/write loop.
//!
//! The library is free of any UI or host-framework dependencies. Consumers
//! receive data and lifecycle events through the [`EventSink`] trait.
//!
//! With the `cli` feature (on by default), the [`cli`] module provides
//! terminal-integration helpers ([`cli::run_shell`], [`cli::run_idle`])
//! that the `sssh` binary uses — so embedders can reuse the same behavior
//! without pulling in the argument parser.
//!
//! ## Features
//!
//! - **Configurable host-key verification** via [`HostKeyVerifier`] trait.
//!   Defaults to [`AcceptAll`] (insecure). Implement the trait for TOFU or
//!   known-hosts-file verification.
//! - **Connection timeout** via [`Config::timeout_secs`].
//! - **Shell quoting** via [`shell_quote`] for safe command construction.
//! - **Command output** as a [`std::process::Output`]-shaped [`Output`]
//!   (raw `stdout`/`stderr` bytes plus an [`ExitStatus`]), with streaming
//!   variants that write straight to any `AsyncWrite`.
//! - **Remote filesystem API** via [`fs`] — a [`std::fs`]-like layer
//!   (`read`, `write`, `create_dir_all`, `try_exists`, `metadata`,
//!   `read_dir`, `symlink`, `download`, `upload`, …) on top of [`Client`].
//!
//! ## Quick start — remote files
//!
//! ```no_run
//! # async fn run(config: sssh::Config) -> Result<(), Box<dyn std::error::Error>> {
//! use sssh::Client;
//!
//! let client = Client::connect(&config).await?;
//! let fs = client.fs();
//!
//! fs.create_dir_all("~/.local/bin").await?;
//! fs.write("~/.local/bin/hello.txt", b"hello over ssh\n").await?;
//! fs.set_mode("~/.local/bin/hello.txt", 0o644).await?;
//!
//! println!("{}", fs.read_to_string("~/.local/bin/hello.txt").await?);
//! println!("free: {:?}", fs.available_space("~").await?);
//! # Ok(()) }
//! ```
//!
//! ## Quick start — run a command and get the result
//!
//! ```no_run
//! # async fn run() -> Result<(), sssh::Error> {
//! let out = sssh::exec("user@host", 22, sssh::Auth::password("pass"), "uname -a").await?;
//! println!("{}", out.stdout_lossy());
//! println!("exit: {:?}", out.code());
//! # Ok(()) }
//! ```
//!
//! Or with a private key:
//!
//! ```no_run
//! # async fn run() -> Result<(), sssh::Error> {
//! let key = std::fs::read_to_string("~/.ssh/id_rsa")?;
//! let out = sssh::exec("user@host", 22, sssh::Auth::key(key), "ls -la").await?;
//! println!("{}", out.stdout_lossy());
//! # Ok(()) }
//! ```
//!
//! ## Quick start — interactive shell
//!
//! ```no_run
//! use sssh::{Config, EventSink, Session};
//!
//! struct MySink;
//! impl EventSink for MySink {
//!     fn on_connected(&self) {}
//!     fn on_data(&self, data: String) { print!("{data}"); }
//!     fn on_closed(&self) {}
//! }
//!
//! # async fn run() -> Result<(), sssh::Error> {
//! let config = Config {
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
//! };
//! let mut session = Session::connect(config, MySink).await?;
//! session.write("ls\n".as_bytes())?;
//! # Ok(()) }
//! ```

mod client;
mod error;
pub mod fs;
mod handler;
mod io_loop;
mod output;
mod session;
mod shell;

#[cfg(feature = "cli")]
pub mod cli;

pub use client::Client;
pub use error::Error;
pub use fs::Fs;
pub use handler::{AcceptAll, HostKeyVerifier};
pub use output::{ExitStatus, Output};
pub use session::{Auth, Config, EventSink, Session, exec, parse_target};
pub use shell::shell_quote;
