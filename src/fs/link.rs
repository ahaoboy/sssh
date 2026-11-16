//! Symbolic and hard links.
//!
//! Mirrors [`std::fs::symlink`] / [`std::fs::hard_link`] /
//! [`std::fs::read_link`] (`std::fs::symlink` itself is Unix-only in std).

use std::io;
use std::path::{Path, PathBuf};

use super::{Fs, quote};

impl Fs {
    /// Create a symbolic link at `link` pointing to `original`.
    ///
    /// `original` is not resolved against `$HOME`: a relative target is
    /// interpreted relative to the link's directory by the remote host,
    /// exactly like `ln -s`.
    pub async fn symlink(
        &self,
        original: impl AsRef<Path>,
        link: impl AsRef<Path>,
    ) -> io::Result<()> {
        let original = original.as_ref().to_string_lossy().into_owned();
        let link = self.resolve(link);
        let script = format!("ln -s {} {}", quote(&original), quote(&link));
        self.run_unit("symlink", &link, &script).await
    }

    /// Create a hard link at `link` pointing to `original`.
    pub async fn hard_link(
        &self,
        original: impl AsRef<Path>,
        link: impl AsRef<Path>,
    ) -> io::Result<()> {
        let original = self.resolve(original);
        let link = self.resolve(link);
        let script = format!("ln {} {}", quote(&original), quote(&link));
        self.run_unit("hard_link", &link, &script).await
    }

    /// Read the target of a symbolic link, like [`std::fs::read_link`].
    pub async fn read_link(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        let target = self.resolve(path);
        let script = format!("readlink {}", quote(&target));
        let out = self.run_checked("read_link", &target, &script).await?;
        let link = out.stdout_lossy();
        let link = link.trim_end_matches(['\r', '\n']);
        if link.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("'{target}' is not a symbolic link"),
            ));
        }
        Ok(PathBuf::from(link))
    }
}
