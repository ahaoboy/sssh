//! Symbolic and hard links.
//!
//! Mirrors [`std::fs::symlink`] / [`std::fs::hard_link`] /
//! [`std::fs::read_link`] (`std::fs::symlink` itself is Unix-only in std),
//! except that creating a link replaces an existing entry so an install can be
//! repeated.

use std::io;
use std::path::{Path, PathBuf};

use super::{Fs, quote};

/// Command that makes `link` point at `original`.
///
/// `-f` replaces an existing entry and `-n` stops a symlink to a directory
/// from being used as the destination directory; `--` keeps paths that start
/// with `-` from being read as options.
fn symlink_command(original: &str, link: &str) -> String {
    format!("ln -sfn -- {} {}", quote(original), quote(link))
}

impl Fs {
    /// Create a symbolic link at `link` pointing to `original`.
    ///
    /// `original` is not resolved against `$HOME`: a relative target is
    /// interpreted relative to the link's directory by the remote host,
    /// exactly like `ln -s`.
    ///
    /// An existing entry at `link` is replaced instead of being written
    /// through (like `ln -sfn`), so installing twice is safe. This is the one
    /// difference from [`std::fs::symlink`], which fails with
    /// [`io::ErrorKind::AlreadyExists`]. Parent directories are not created,
    /// matching [`std::fs::symlink`]; call [`Fs::create_dir_all`] first.
    pub async fn symlink(
        &self,
        original: impl AsRef<Path>,
        link: impl AsRef<Path>,
    ) -> io::Result<()> {
        let original = original.as_ref().to_string_lossy().into_owned();
        let link = self.resolve(link);
        let script = symlink_command(&original, &link);
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

#[cfg(test)]
mod tests {
    use super::symlink_command;

    /// The command has to replace any existing entry, ignore `-` prefixes and
    /// keep both paths quoted.
    #[test]
    fn symlink_command_replaces_and_quotes() {
        let command = symlink_command("libfoo.so.1.2.3", "/opt/my tools/libfoo.so.1");

        assert_eq!(
            command,
            "ln -sfn -- 'libfoo.so.1.2.3' '/opt/my tools/libfoo.so.1'"
        );
    }
}
