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
//!
//! ## Quick start — run a command and get the result
//!
//! ```no_run
//! # async fn run() -> Result<(), sssh::Error> {
//! let out = sssh::exec("user@host", 22, sssh::Auth::password("pass"), "uname -a").await?;
//! println!("{}", out.stdout);
//! println!("exit: {:?}", out.exit_code);
//! # Ok(()) }
//! ```
//!
//! Or with a private key:
//!
//! ```no_run
//! # async fn run() -> Result<(), sssh::Error> {
//! let key = std::fs::read_to_string("~/.ssh/id_rsa")?;
//! let out = sssh::exec("user@host", 22, sssh::Auth::key(key), "ls -la").await?;
//! println!("{}", out.stdout);
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

mod error;
mod handler;
mod io_loop;
mod session;

#[cfg(feature = "cli")]
pub mod cli;

pub use error::Error;
pub use handler::{AcceptAll, HostKeyVerifier};
pub use session::{Auth, Config, EventSink, Output, Session, exec, parse_target};

/// Quote a string for safe inclusion as a single shell argument (POSIX-style
/// single-quote wrapping: `'` → `'\''`).
///
/// This is a re-export of [`cli::input::shell_quote`] — it lives at the crate
/// root for convenience since shell quoting is a library-level concern, not
/// just a CLI one.
#[cfg(feature = "cli")]
pub use cli::shell_quote;
