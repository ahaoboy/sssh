//! Disk space and directory usability checks.
//!
//! Backed by `src/fs/scripts/dir_status.sh`, which reports whether a
//! directory exists/writable/creatable and how much space the filesystem
//! holding it has. `stat -f` is preferred; `df -Pk` is parsed as a fallback
//! (BusyBox prints only the header when a path is missing, so the raw table
//! is returned instead of the last line).
//!
//! The same probe is reused by [`Fs::dir_status`], [`Fs::disk_space`] and
//! [`Fs::available_space`], so platform quirks live in exactly one place.

use std::io;
use std::path::{Path, PathBuf};

use super::parse;
use super::types::{DirStatus, DiskSpace};
use super::{Fs, quote};

const DIR_STATUS_SCRIPT: &str = include_str!("scripts/dir_status.sh");

impl Fs {
    /// Inspect a directory: exists / writable / creatable plus disk space.
    ///
    /// When the path does not exist yet, the nearest existing ancestor is
    /// probed, so the result is useful before the first write.
    pub async fn dir_status(&self, path: impl AsRef<Path>) -> io::Result<DirStatus> {
        let target = self.resolve(path);
        let script = super::probe(DIR_STATUS_SCRIPT).replace("__DIR__", &quote(&target));
        let out = self.run_checked("dir_status", &target, &script).await?;
        Ok(DirStatus::parse(target, &out.stdout_lossy()))
    }

    /// Total/available space of the filesystem that holds `path`.
    ///
    /// [`DiskSpace::known`] is `false` when the host cannot report space.
    pub async fn disk_space(&self, path: impl AsRef<Path>) -> io::Result<DiskSpace> {
        Ok(self.dir_status(path).await?.space)
    }

    /// Bytes available on the filesystem that holds `path`.
    ///
    /// Returns `Ok(None)` when the host cannot report it; a genuine `0`
    /// (full filesystem) is returned as `Some(0)`.
    pub async fn available_space(&self, path: impl AsRef<Path>) -> io::Result<Option<u64>> {
        let space = self.disk_space(path).await?;
        Ok(space.known.then_some(space.available_bytes))
    }

    /// Total size in bytes of the filesystem that holds `path`.
    ///
    /// Returns `Ok(None)` when the host cannot report it.
    pub async fn total_space(&self, path: impl AsRef<Path>) -> io::Result<Option<u64>> {
        let space = self.disk_space(path).await?;
        Ok(space.known.then_some(space.total_bytes))
    }
}

impl DirStatus {
    pub(crate) fn parse(path: String, stdout: &str) -> Self {
        let exists = parse::flag(stdout, "EXISTS", "yes");
        let writable = parse::flag(stdout, "WRITABLE", "yes");
        let creatable = parse::flag(stdout, "CREATABLE", "yes");

        let probe = parse::marker(stdout, "PROBE")
            .filter(|probe| !probe.is_empty())
            .unwrap_or(&path)
            .to_string();
        let mut space = DiskSpace {
            known: false,
            probe_path: PathBuf::from(&probe),
            ..DiskSpace::default()
        };

        // `stat -f` markers (preferred, unambiguous), else the `df` table.
        if let (Some(total), Some(available), Some(size)) = (
            parse::num(stdout, "TOTAL_BLOCKS"),
            parse::num(stdout, "AVAIL_BLOCKS"),
            parse::num(stdout, "BLOCK_SIZE"),
        ) {
            if size > 0 {
                space.total_bytes = total.saturating_mul(size);
                space.available_bytes = available.saturating_mul(size);
                space.known = true;
            }
        } else if let Some((total, available)) = parse::parse_df(stdout) {
            space.total_bytes = total;
            space.available_bytes = available;
            space.known = true;
        }

        let error = if !exists && !creatable {
            Some("directory does not exist and cannot be created".to_string())
        } else if exists && !writable {
            Some("directory is not writable".to_string())
        } else {
            None
        };

        Self {
            path: PathBuf::from(path),
            exists,
            writable,
            creatable,
            space,
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_space_markers() {
        let out = "EXISTS=yes\nWRITABLE=yes\nPROBE=/home/u\n\
                   TOTAL_BLOCKS=1000\nAVAIL_BLOCKS=400\nBLOCK_SIZE=4096\n";
        let status = DirStatus::parse("/home/u".into(), out);
        assert!(status.exists && status.writable);
        assert!(status.usable());
        assert_eq!(status.error, None);
        assert!(status.space.known);
        assert_eq!(status.space.total_bytes, 1000 * 4096);
        assert_eq!(status.space.available_bytes, 400 * 4096);
        assert_eq!(status.space.probe_path, PathBuf::from("/home/u"));
    }

    #[test]
    fn falls_back_to_df_when_stat_is_missing() {
        let out = "EXISTS=yes\nWRITABLE=yes\nPROBE=/home/u\n\
                   Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                   /dev/sda1 1000 400 600 40% /\n";
        let status = DirStatus::parse("/home/u".into(), out);
        assert!(status.space.known);
        assert_eq!(status.space.total_bytes, 1000 * 1024);
        assert_eq!(status.space.available_bytes, 600 * 1024);
    }

    #[test]
    fn header_only_df_is_unknown_space() {
        let out = "EXISTS=yes\nWRITABLE=yes\nPROBE=/home/u\n\
                   Filesystem 1024-blocks Used Available Capacity Mounted on\n";
        let status = DirStatus::parse("/home/u".into(), out);
        assert!(!status.space.known);
        assert_eq!(status.space.available_bytes, 0);
    }

    #[test]
    fn full_filesystem_is_known_and_zero() {
        let out = "EXISTS=yes\nWRITABLE=yes\nPROBE=/home/u\n\
                   TOTAL_BLOCKS=1000\nAVAIL_BLOCKS=0\nBLOCK_SIZE=4096\n";
        let status = DirStatus::parse("/home/u".into(), out);
        assert!(status.space.known);
        assert_eq!(status.space.available_bytes, 0);
    }

    #[test]
    fn creatable_directory_is_usable() {
        let out = "CREATABLE=yes\nPROBE=/home/u\nTOTAL_BLOCKS=1\nAVAIL_BLOCKS=1\nBLOCK_SIZE=512\n";
        let status = DirStatus::parse("/home/u/new".into(), out);
        assert!(!status.exists);
        assert!(status.creatable);
        assert!(status.usable());
        assert_eq!(status.error, None);
        assert_eq!(status.space.probe_path, PathBuf::from("/home/u"));
    }

    #[test]
    fn unusable_directory_reports_error() {
        let out = "EXISTS=yes\nPROBE=/root\nTOTAL_BLOCKS=1\nAVAIL_BLOCKS=1\nBLOCK_SIZE=512\n";
        let status = DirStatus::parse("/root".into(), out);
        assert!(!status.writable);
        assert!(!status.usable());
        assert_eq!(status.error.as_deref(), Some("directory is not writable"));
    }
}
