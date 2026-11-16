//! Writing files.
//!
//! Every write is verified by size: a full disk makes `cat` exit early, and
//! without the check a caller would silently get a truncated file. The
//! expected byte count is compared with `wc -c` on the remote side.

use std::io;
use std::path::Path;

use super::{Fs, quote};

impl Fs {
    /// Write `contents` to `path`, truncating any existing file.
    ///
    /// Parent directories are **not** created (matching [`std::fs::write`]);
    /// call [`Fs::create_dir_all`] first when needed.
    pub async fn write(
        &self,
        path: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> io::Result<()> {
        self.write_with_mode(path, contents, None).await
    }

    /// Write `contents` to `path` and, when `mode` is set, apply those POSIX
    /// permission bits (e.g. `Some(0o755)`).
    pub async fn write_with_mode(
        &self,
        path: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
        mode: Option<u32>,
    ) -> io::Result<()> {
        let target = self.resolve(path);
        let data = contents.as_ref();
        let expected = data.len() as u64;
        let quoted = quote(&target);

        let mut script = format!("cat > {quoted}");
        if let Some(mode) = mode.filter(|mode| *mode > 0) {
            script.push_str(&format!(" && chmod {:o} {quoted}", mode & 0o7777));
        }
        // Report the resulting size so a short write is detected below.
        script.push_str(&format!(" && wc -c < {quoted} | tr -d ' \\n'"));

        let out = self
            .client()
            .exec_with_input(&script, data)
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
        self.check_written("write", &target, expected, &out)
    }

    /// Append `contents` to `path`, creating the file when it does not exist.
    pub async fn append(
        &self,
        path: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> io::Result<()> {
        let target = self.resolve(path);
        let existing = match self.try_metadata(&target).await? {
            Some(meta) if meta.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("'{target}' is a directory"),
                ));
            }
            Some(meta) => meta.len(),
            None => 0,
        };
        let data = contents.as_ref();
        let expected = existing + data.len() as u64;
        let quoted = quote(&target);
        let script = format!("cat >> {quoted} && wc -c < {quoted} | tr -d ' \\n'");

        let out = self
            .client()
            .exec_with_input(&script, data)
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
        self.check_written("append", &target, expected, &out)
    }

    /// Create `path` as an empty file (truncating an existing one), like
    /// [`std::fs::File::create`].
    pub async fn create(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let target = self.resolve(path);
        let script = format!(": > {}", quote(&target));
        self.run_unit("create", &target, &script).await
    }

    /// Create `path` when missing and update its mtime, without truncating
    /// existing contents (like `touch(1)`).
    pub async fn touch(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let target = self.resolve(path);
        let script = format!("touch {}", quote(&target));
        self.run_unit("touch", &target, &script).await
    }

    fn check_written(
        &self,
        op: &str,
        target: &str,
        expected: u64,
        out: &crate::output::Output,
    ) -> io::Result<()> {
        if !out.success() {
            return Err(self.failure(op, target, out));
        }
        let written: u64 = out.stdout_lossy().trim().parse().unwrap_or(0);
        if written != expected {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!(
                    "wrote {written} of {expected} bytes to '{target}' on {} (disk full?)",
                    self.client().label()
                ),
            ));
        }
        Ok(())
    }
}
