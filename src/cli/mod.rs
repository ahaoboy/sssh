//! CLI runtime: the interactive/one-shot execution logic shared by the
//! `sssh` binary and any embedder that wants the same behavior.
//!
//! This module depends on `crossterm` (for raw-mode terminal handling) and
//! `rpassword` (for interactive password prompts), both gated by the
//! `cli` feature. It is intentionally free of argument-parsing concerns.
//!
//! ## Two layers
//!
//! - **Generic protocol API** — [`crate::Session`] provides
//!   [`connect`](crate::Session::connect),
//!   [`connect_idle`](crate::Session::connect_idle),
//!   [`exec`](crate::Session::exec) (returns [`Output`](crate::Output)
//!   with no terminal side effects), and
//!   [`exec_stream`](crate::Session::exec_stream) (streams to any writer).
//!   Use these directly for programmatic / non-interactive use.
//!
//! - **Terminal helpers** — [`run_shell`] and [`run_idle`] add raw-mode,
//!   stdin forwarding, and Ctrl-C handling for terminal-based use.

pub mod fs;
mod input;
mod run;

pub use input::{Input, Mode, build_config, credentials, resolve_command, shell_quote};
pub use run::{run_cmd, run_idle, run_shell};
