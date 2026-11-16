//! Creating, removing, renaming, copying and linking.
//!
//! All operations map to POSIX utilities (`mkdir`, `rm`, `rmdir`, `mv`,
//! `cp`, `ln`) so they behave like their [`std::fs`] counterparts on the
//! remote host — including returning [`io::ErrorKind::NotFound`] instead of
//! silently succeeding.

use std::io;
use std::path::Path;

use super::{Fs, quote};

impl Fs {
    /// Create the parent directories of `path`, if any.
    ///
    /// Convenience for install-style code that writes into a directory tree
    /// which may not exist yet (`std::fs::write` itself never creates
    /// parents):
    ///
    /// ```no_run
    /// # async fn run(fs: sssh::Fs) -> std::io::Result<()> {
    /// fs.create_parent_dirs("~/.local/bin/tool").await?;
    /// fs.write("~/.local/bin/tool", b"...").await?;
    /// # Ok(()) }
    /// ```
    pub async fn create_parent_dirs(&self, path: impl AsRef<Path>) -> io::Result<()> {
        match super::path::parent_dir(&self.resolve(path)) {
            Some(parent) => self.create_dir_all(parent).await,
            None => Ok(()),
        }
    }

    /// Create a directory (without parents), like [`std::fs::create_dir`].
    pub async fn create_dir(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let target = self.resolve(path);
        let script = format!("mkdir {}", quote(&target));
        self.run_unit("create_dir", &target, &script).await
    }

    /// Create a directory and all missing parents, like
    /// [`std::fs::create_dir_all`].
    ///
    /// Succeeds when the directory already exists.
    pub async fn create_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let target = self.resolve(path);
        let script = format!("mkdir -p {}", quote(&target));
        self.run_unit("create_dir_all", &target, &script).await
    }

    /// Remove a file, like [`std::fs::remove_file`].
    ///
    /// Returns [`io::ErrorKind::NotFound`] when the file does not exist.
    pub async fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let target = self.resolve(path);
        let script = format!("rm {}", quote(&target));
        self.run_unit("remove_file", &target, &script).await
    }

    /// Remove an empty directory, like [`std::fs::remove_dir`].
    pub async fn remove_dir(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let target = self.resolve(path);
        let script = format!("rmdir {}", quote(&target));
        self.run_unit("remove_dir", &target, &script).await
    }

    /// Remove a directory and everything below it, like
    /// [`std::fs::remove_dir_all`].
    ///
    /// Returns [`io::ErrorKind::NotFound`] when the path does not exist.
    pub async fn remove_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let target = self.resolve(path);
        if self.try_symlink_metadata(&target).await?.is_none() {
            return Err(self.missing(&target));
        }
        let script = format!("rm -rf {}", quote(&target));
        self.run_unit("remove_dir_all", &target, &script).await
    }

    /// Rename a file or directory, like [`std::fs::rename`].
    ///
    /// Overwrites an existing destination file, matching the POSIX `mv -f`
    /// behaviour that `std::fs::rename` also has on Unix.
    pub async fn rename(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let from = self.resolve(from);
        let to = self.resolve(to);
        let script = format!("mv -f {} {}", quote(&from), quote(&to));
        self.run_unit("rename", &from, &script).await
    }

    /// Copy a file (preserving mode and timestamps), like
    /// [`std::fs::copy`].
    pub async fn copy(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let from = self.resolve(from);
        let to = self.resolve(to);
        let script = format!("cp -p {} {}", quote(&from), quote(&to));
        self.run_unit("copy", &from, &script).await
    }

    /// Resolve `path` to an absolute, canonical path with `..` and symlinks
    /// removed, like [`std::fs::canonicalize`].
    pub async fn canonicalize(&self, path: impl AsRef<Path>) -> io::Result<std::path::PathBuf> {
        let target = self.resolve(path);
        let script = super::probe(CANONICALIZE_SCRIPT).replace("__PATH__", &quote(&target));
        let out = self.run_checked("canonicalize", &target, &script).await?;
        let resolved = out.stdout_lossy();
        let resolved = resolved.trim_end_matches(['\r', '\n']);
        if resolved.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("canonicalize '{target}' returned no path"),
            ));
        }
        Ok(std::path::PathBuf::from(resolved))
    }
}

const CANONICALIZE_SCRIPT: &str = r#"p=__PATH__
if command -v readlink >/dev/null 2>&1; then
  resolved=$(readlink -f "$p" 2>/dev/null) || resolved=
  if [ -n "$resolved" ]; then
    printf '%s\n' "$resolved"
    exit 0
  fi
fi
if [ -d "$p" ]; then
  cd "$p" 2>/dev/null && pwd -P
  exit $?
fi
if [ -e "$p" ] || [ -L "$p" ]; then
  d=$(dirname "$p")
  b=$(basename "$p")
  cd "$d" 2>/dev/null && printf '%s/%s\n' "$(pwd -P)" "$b"
  exit $?
fi
echo "canonicalize: $p: No such file or directory" >&2
exit 1"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_script_substitutes_path() {
        let script = crate::fs::probe(CANONICALIZE_SCRIPT).replace("__PATH__", &quote("/tmp/a"));
        assert!(!script.contains("__PATH__"));
        assert!(script.contains("p='/tmp/a'"));
    }
}
