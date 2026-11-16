//! Remote filesystem access with a [`std::fs`]-like API.
//!
//! [`Fs`] wraps an authenticated [`Client`] and implements the same
//! vocabulary as [`std::fs`] — [`read`](Fs::read), [`write`](Fs::write),
//! [`create_dir_all`](Fs::create_dir_all), [`metadata`](Fs::metadata),
//! [`read_dir`](Fs::read_dir), [`rename`](Fs::rename), … — except that every
//! call is `async` and addresses the **remote** host.
//!
//! ```no_run
//! # async fn run(client: sssh::Client) -> std::io::Result<()> {
//! let fs = client.fs();
//!
//! fs.create_dir_all("~/.config/app").await?;
//! fs.write("~/.config/app/config.toml", b"key = 'value'\n").await?;
//!
//! let text = fs.read_to_string("~/.config/app/config.toml").await?;
//! assert!(text.contains("key"));
//!
//! for entry in fs.read_dir("~/.config/app").await? {
//!     let entry = entry?;
//!     println!("{}", entry.path().display());
//! }
//!
//! if let Some(free) = fs.available_space("~").await? {
//!     println!("{free} bytes free");
//! }
//! # Ok(()) }
//! ```
//!
//! ## Paths
//!
//! Paths are POSIX paths. `~` expands to the remote `$HOME`, and relative
//! paths resolve against it (the directory a non-interactive SSH command
//! starts in). They are **not** interpreted on the local machine, so
//! `C:\…`-style local paths are meaningless here.
//!
//! ## Layout
//!
//! The implementation is split by concern so new features can be added
//! without editing one giant file:
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | `path` | POSIX path helpers (`~` expansion, parent lookup) |
//! | `read` | reading files and directories |
//! | `write` | writing and appending files |
//! | `dir` | creating, removing, renaming and copying |
//! | `link` | symbolic and hard links |
//! | `stat` | `metadata` |
//! | `chmod` | changing permission bits |
//! | `space` | disk space and [`DirStatus`] |
//! | `stream` | streaming downloads (`std::io::copy`-style) |
//! | `parse` | parsing the `KEY=value` output of shell probes |
//! | `types` | std-like value types (`Metadata`, `DirEntry`, …) |
//!
//! Shell probes live in `src/fs/scripts/` and are embedded with
//! [`include_str!`], so they can be run and tested by hand.

mod chmod;
mod dir;
mod link;
mod parse;
mod path;
mod read;
mod space;
mod stat;
mod stream;
mod types;
mod write;

pub use types::{
    DirEntry, DirStatus, DiskSpace, FileKind, FileType, Metadata, Permissions, ReadDir,
};

pub(crate) use path::expand_with_home;

use std::io;
use std::path::Path;

use crate::client::Client;
use crate::output::Output;
use crate::shell::shell_quote;

/// Filesystem operations on a remote host.
///
/// Created with [`Client::fs`] or [`Fs::new`]. Cloning is cheap and shares
/// the underlying connection.
#[derive(Clone)]
pub struct Fs {
    client: Client,
}

impl std::fmt::Debug for Fs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fs")
            .field("host", &self.client.label())
            .finish()
    }
}

impl Fs {
    /// Bind a filesystem handle to `client`.
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// The connection this handle operates on.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Resolve `path` to an absolute remote path (`~` → `$HOME`, relative
    /// paths → `$HOME/…`).
    pub(crate) fn resolve(&self, path: impl AsRef<Path>) -> String {
        path::expand_with_home(&path.as_ref().to_string_lossy(), self.client.home())
    }

    /// Run a shell script and capture its output, regardless of exit status.
    pub(crate) async fn run(&self, script: &str) -> io::Result<Output> {
        self.client.exec_sh(script).await.map_err(|e| match e {
            crate::Error::Timeout(_) => io::Error::new(io::ErrorKind::TimedOut, e.to_string()),
            e => io::Error::other(e.to_string()),
        })
    }

    /// Run a shell script, turning a non-zero exit status into an
    /// [`io::Error`] with the remote stderr as the message.
    pub(crate) async fn run_checked(
        &self,
        op: &str,
        target: &str,
        script: &str,
    ) -> io::Result<Output> {
        let out = self.run(script).await?;
        if !out.success() {
            return Err(self.failure(op, target, &out));
        }
        Ok(out)
    }

    /// Like [`Self::run_checked`], for commands whose output is not needed.
    pub(crate) async fn run_unit(&self, op: &str, target: &str, script: &str) -> io::Result<()> {
        self.run_checked(op, target, script).await.map(|_| ())
    }

    /// Build the [`io::Error`] for a failed remote command.
    pub(crate) fn failure(&self, op: &str, target: &str, out: &Output) -> io::Error {
        let stderr = out.stderr_lossy();
        let detail = if stderr.trim().is_empty() {
            out.status.to_string()
        } else {
            stderr.trim().to_string()
        };
        io::Error::new(
            error_kind(&stderr),
            format!(
                "{op} '{target}' failed on {}: {detail}",
                self.client.label()
            ),
        )
    }

    /// The [`io::Error`] for a path that does not exist.
    pub(crate) fn missing(&self, target: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("'{target}' does not exist on {}", self.client.label()),
        )
    }
}

/// Map common remote error messages to [`io::ErrorKind`]s so callers can
/// branch on `Error::kind()` just like with [`std::fs`].
fn error_kind(stderr: &str) -> io::ErrorKind {
    let msg = stderr.to_ascii_lowercase();
    if msg.contains("no such file")
        || msg.contains("not found")
        || msg.contains("does not exist")
        || msg.contains("no such")
    {
        io::ErrorKind::NotFound
    } else if msg.contains("permission denied")
        || msg.contains("operation not permitted")
        || msg.contains("read-only file system")
    {
        io::ErrorKind::PermissionDenied
    } else if msg.contains("file exists") {
        io::ErrorKind::AlreadyExists
    } else if msg.contains("not a directory") || msg.contains("is a directory") {
        io::ErrorKind::InvalidInput
    } else {
        io::ErrorKind::Other
    }
}

/// Quote a path for interpolation into a remote command.
pub(crate) fn quote(path: &str) -> String {
    shell_quote(path)
}

/// Load an embedded shell probe with POSIX (`\n`) line endings.
///
/// The scripts are checked in with `eol=lf`, but a Windows checkout can still
/// hand `include_str!` a CRLF copy; a stray `\r` makes `sh` fail with
/// `\r: not found`, so normalize defensively.
pub(crate) fn probe(body: &str) -> String {
    body.replace("\r\n", "\n")
}

/// Join a remote directory and a relative name with `/` (never `\`, even on
/// Windows hosts).
pub(crate) fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_errors() {
        assert_eq!(
            error_kind("cat: /x: No such file or directory"),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            error_kind("mkdir: /x: Permission denied"),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            error_kind("mkdir: /x: File exists"),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(error_kind("something odd"), io::ErrorKind::Other);
    }

    #[test]
    fn normalizes_script_line_endings() {
        assert_eq!(probe("a\r\nb\r\n"), "a\nb\n");
    }

    #[test]
    fn joins_remote_paths() {
        assert_eq!(join_remote("/home/u", "f"), "/home/u/f");
        assert_eq!(join_remote("/", "f"), "/f");
    }
}
