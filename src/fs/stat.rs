//! `metadata` (and the permissions it carries).
//!
//! Backed by `src/fs/scripts/stat.sh`, which prefers GNU/BusyBox
//! `stat -c`, falls back to BSD/macOS `stat -f`, and finally to a coarse
//! `TYPE=` probe on hosts without a usable `stat`.

use std::io;
use std::path::Path;

use super::types::Metadata;
use super::{Fs, quote};

const STAT_SCRIPT: &str = include_str!("scripts/stat.sh");

impl Fs {
    /// Metadata for `path`, following symlinks (like [`std::fs::metadata`]).
    pub async fn metadata(&self, path: impl AsRef<Path>) -> io::Result<Metadata> {
        let target = self.resolve(path);
        self.probe_metadata(&target, true)
            .await?
            .ok_or_else(|| self.missing(&target))
    }

    /// Metadata for `path` without following symlinks (like
    /// [`std::fs::symlink_metadata`]).
    pub async fn symlink_metadata(&self, path: impl AsRef<Path>) -> io::Result<Metadata> {
        let target = self.resolve(path);
        self.probe_metadata(&target, false)
            .await?
            .ok_or_else(|| self.missing(&target))
    }

    /// Like [`Self::metadata`], but returns `Ok(None)` when the path is
    /// missing instead of an error.
    pub async fn try_metadata(&self, path: impl AsRef<Path>) -> io::Result<Option<Metadata>> {
        let target = self.resolve(path);
        self.probe_metadata(&target, true).await
    }

    /// Like [`Self::symlink_metadata`], but returns `Ok(None)` when the path
    /// is missing instead of an error.
    pub async fn try_symlink_metadata(
        &self,
        path: impl AsRef<Path>,
    ) -> io::Result<Option<Metadata>> {
        let target = self.resolve(path);
        self.probe_metadata(&target, false).await
    }

    async fn probe_metadata(&self, target: &str, deref: bool) -> io::Result<Option<Metadata>> {
        let script = super::probe(STAT_SCRIPT)
            .replace("__PATH__", &quote(target))
            .replace("__FOLLOW_MODE__", if deref { "yes" } else { "no" })
            .replace("__STAT_FLAG__", if deref { "-L" } else { "" });
        let out = self.run_checked("metadata", target, &script).await?;
        let text = out.stdout_lossy();
        Metadata::parse(&text, deref)
            .map_err(|e| io::Error::new(e.kind(), format!("{e} ('{target}')")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::parse;

    fn build_script(target: &str, deref: bool) -> String {
        crate::fs::probe(STAT_SCRIPT)
            .replace("__PATH__", &quote(target))
            .replace("__FOLLOW_MODE__", if deref { "yes" } else { "no" })
            .replace("__STAT_FLAG__", if deref { "-L" } else { "" })
    }

    #[test]
    fn substitutes_placeholders() {
        let script = build_script("/tmp/a b", true);
        assert!(!script.contains("__PATH__"));
        assert!(!script.contains("__FOLLOW_MODE__"));
        assert!(!script.contains("__STAT_FLAG__"));
        assert!(script.contains("path='/tmp/a b'"));
        assert!(script.contains("statflag='-L'"));
    }

    #[test]
    fn marks_symlink_mode() {
        let script = build_script("/tmp/a", false);
        assert!(script.contains("follow=no"));
        assert!(script.contains("statflag=''"));
    }

    #[test]
    fn parses_probe_output_through_helper() {
        let out = "STAT=regular file|12|600|0|0|1700000000|9\n";
        let meta = parse::flag(out, "NOTFOUND", "1");
        assert!(!meta);
        assert_eq!(Metadata::parse(out, true).unwrap().unwrap().len(), 12);
    }
}
