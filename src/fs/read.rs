//! Reading files and directories.
//!
//! [`Fs::read`] streams raw bytes through `cat`, so it is binary-safe. For
//! large files prefer [`Fs::download`](Fs::download), which never buffers the
//! payload. [`Fs::read_dir`] lists a directory with a single command and
//! parses the NUL-separated output of `find` (falling back to `ls -1A` on
//! hosts without `find`).

use std::io;
use std::path::{Path, PathBuf};

use super::types::{DirEntry, ReadDir};
use super::{Fs, quote};

const READ_DIR_SCRIPT: &str = include_str!("scripts/read_dir.sh");

impl Fs {
    /// Read the entire contents of `path` as raw bytes.
    pub async fn read(&self, path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let target = self.resolve(path);
        let script = format!("cat {}", quote(&target));
        let out = self.run_checked("read", &target, &script).await?;
        Ok(out.stdout)
    }

    /// Read the entire contents of `path` as UTF-8.
    pub async fn read_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        let target = self.resolve(path);
        let bytes = self.read(&target).await?;
        String::from_utf8(bytes).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("'{target}' is not valid UTF-8: {e}"),
            )
        })
    }

    /// Whether `path` exists, like `std::fs::try_exists`.
    ///
    /// A broken symlink counts as existing, because the link itself is
    /// checked as well as its target.
    pub async fn try_exists(&self, path: impl AsRef<Path>) -> io::Result<bool> {
        let target = self.resolve(path);
        let quoted = quote(&target);
        // A marker keeps the answer independent of the exit status, which
        // some servers omit (making every command look successful).
        let script =
            format!("if [ -e {quoted} ] || [ -L {quoted} ]; then echo yes; else echo no; fi");
        let out = self.run_checked("try_exists", &target, &script).await?;
        Ok(out.stdout_lossy().trim() == "yes")
    }

    /// Whether `path` exists.
    ///
    /// Convenience alias for [`Self::try_exists`].
    pub async fn exists(&self, path: impl AsRef<Path>) -> io::Result<bool> {
        self.try_exists(path).await
    }

    /// Whether `path` is a directory.
    pub async fn is_dir(&self, path: impl AsRef<Path>) -> io::Result<bool> {
        self.test("is_dir", path, "-d").await
    }

    /// Whether `path` is a regular file.
    pub async fn is_file(&self, path: impl AsRef<Path>) -> io::Result<bool> {
        self.test("is_file", path, "-f").await
    }

    async fn test(&self, op: &str, path: impl AsRef<Path>, flag: &str) -> io::Result<bool> {
        let target = self.resolve(path);
        let script = format!(
            "if [ {flag} {} ]; then echo yes; else echo no; fi",
            quote(&target)
        );
        let out = self.run_checked(op, &target, &script).await?;
        Ok(out.stdout_lossy().trim() == "yes")
    }

    /// List the entries of a directory (like [`std::fs::read_dir`]).
    ///
    /// Entries are fetched in one round trip; the iterator yields
    /// [`DirEntry`] values whose `metadata()`/`file_type()` query the host
    /// lazily.
    pub async fn read_dir(&self, path: impl AsRef<Path>) -> io::Result<ReadDir> {
        let target = self.resolve(path);
        let script = super::probe(READ_DIR_SCRIPT).replace("__DIR__", &quote(&target));
        let out = self.run(&script).await?;

        let text = out.stdout_lossy();
        if text.contains("ERROR=notfound") {
            return Err(self.missing(&target));
        }
        if text.contains("ERROR=notdir") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("'{target}' is not a directory"),
            ));
        }

        // `find` may be missing or too old for `-mindepth`; `ls` always works.
        if !out.success() || !text.starts_with("MODE=") {
            return self.read_dir_ls(&target).await;
        }

        let Some(header_end) = out.stdout.iter().position(|byte| *byte == b'\n') else {
            return Ok(ReadDir::new(Vec::new()));
        };
        let mode = String::from_utf8_lossy(&out.stdout[..header_end]);
        let body = &out.stdout[header_end + 1..];

        let mut entries = Vec::new();
        if mode.trim() == "MODE=find" {
            for chunk in body.split(|byte| *byte == 0) {
                if chunk.is_empty() {
                    continue;
                }
                let entry = String::from_utf8_lossy(chunk).into_owned();
                entries.push(DirEntry::new(self.clone(), PathBuf::from(entry)));
            }
        } else {
            for line in String::from_utf8_lossy(body).lines() {
                if !line.is_empty() {
                    entries.push(DirEntry::new(
                        self.clone(),
                        PathBuf::from(super::join_remote(&target, line)),
                    ));
                }
            }
        }

        Ok(ReadDir::new(entries))
    }

    async fn read_dir_ls(&self, target: &str) -> io::Result<ReadDir> {
        let script = format!("ls -1A {}", quote(target));
        let out = self.run_checked("read_dir", target, &script).await?;
        let text = out.stdout_lossy();
        let mut entries = Vec::new();
        for line in text.lines() {
            if !line.is_empty() {
                entries.push(DirEntry::new(
                    self.clone(),
                    PathBuf::from(super::join_remote(target, line)),
                ));
            }
        }
        Ok(ReadDir::new(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_dir_script_substitutes_path() {
        let script = crate::fs::probe(READ_DIR_SCRIPT).replace("__DIR__", &quote("/tmp/a b"));
        assert!(!script.contains("__DIR__"));
        assert!(script.contains("dir='/tmp/a b'"));
    }
}
