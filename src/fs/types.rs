//! Value types mirroring the [`std::fs`] types.
//!
//! These are deliberately small, owned structs: a remote `metadata()` may
//! have to run another command, so the std-like `&self` accessors return
//! plain values instead of borrowing a live handle.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::Fs;
use super::parse;

/// The kind of a filesystem entry, as reported by the remote host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link.
    Symlink,
    /// A named pipe (FIFO).
    Fifo,
    /// A Unix socket.
    Socket,
    /// A character device.
    CharDevice,
    /// A block device.
    BlockDevice,
    /// Anything else (unknown or unsupported).
    #[default]
    Other,
}

impl FileKind {
    /// Classify the `%F` / `%HT` output of `stat(1)`.
    fn from_stat(value: &str) -> Self {
        let value = value.trim().to_ascii_lowercase();
        if value.contains("directory") {
            Self::Dir
        } else if value.contains("symbolic") || value == "link" {
            Self::Symlink
        } else if value.contains("fifo") || value.contains("pipe") {
            Self::Fifo
        } else if value.contains("socket") {
            Self::Socket
        } else if value.contains("character") || value.contains("char device") {
            Self::CharDevice
        } else if value.contains("block") {
            Self::BlockDevice
        } else if value.contains("regular") || value == "file" {
            Self::File
        } else {
            Self::Other
        }
    }

    /// Classify the coarse `TYPE=` fallback marker.
    fn from_type_marker(value: &str) -> Self {
        match value.trim() {
            "file" => Self::File,
            "dir" => Self::Dir,
            "link" => Self::Symlink,
            _ => Self::Other,
        }
    }
}

/// Permissions of a remote file (POSIX mode bits).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Permissions {
    mode: u32,
}

impl Permissions {
    /// Build permissions from raw mode bits (e.g. `0o755`).
    pub fn from_mode(mode: u32) -> Self {
        Self {
            mode: mode & 0o7777,
        }
    }

    /// The raw mode bits.
    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// Whether the file is read-only (no write bits set).
    pub fn readonly(&self) -> bool {
        self.mode & 0o222 == 0
    }
}

/// Type of a filesystem entry, as exposed by [`Metadata::file_type`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FileType {
    kind: FileKind,
}

impl FileType {
    /// Wrap a [`FileKind`].
    pub fn new(kind: FileKind) -> Self {
        Self { kind }
    }

    /// The underlying kind.
    pub fn kind(&self) -> FileKind {
        self.kind
    }

    /// Whether this is a regular file.
    pub fn is_file(&self) -> bool {
        self.kind == FileKind::File
    }

    /// Whether this is a directory.
    pub fn is_dir(&self) -> bool {
        self.kind == FileKind::Dir
    }

    /// Whether this is a symbolic link.
    pub fn is_symlink(&self) -> bool {
        self.kind == FileKind::Symlink
    }

    /// Whether this is a named pipe.
    pub fn is_fifo(&self) -> bool {
        self.kind == FileKind::Fifo
    }

    /// Whether this is a Unix socket.
    pub fn is_socket(&self) -> bool {
        self.kind == FileKind::Socket
    }

    /// Whether this is a character device.
    pub fn is_char_device(&self) -> bool {
        self.kind == FileKind::CharDevice
    }

    /// Whether this is a block device.
    pub fn is_block_device(&self) -> bool {
        self.kind == FileKind::BlockDevice
    }
}

/// Metadata of a remote file, like [`std::fs::Metadata`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    kind: FileKind,
    len: u64,
    permissions: Permissions,
    uid: u32,
    gid: u32,
    modified: Option<SystemTime>,
    inode: u64,
}

impl Metadata {
    /// Parse the output of the `stat` probe.
    ///
    /// `deref` mirrors `std::fs::metadata` (follow symlinks) versus
    /// `std::fs::symlink_metadata` (report the link itself). Returns
    /// `Ok(None)` for a path that does not exist.
    pub(crate) fn parse(stdout: &str, deref: bool) -> io::Result<Option<Self>> {
        if parse::flag(stdout, "NOTFOUND", "1") {
            return Ok(None);
        }

        let mut meta = Self::default();
        let mut found = false;

        if let Some(line) = stdout.lines().find_map(|l| l.strip_prefix("STAT=")) {
            let fields: Vec<&str> = line.split('|').collect();
            if fields.len() < 6 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("failed to stat: malformed output '{line}'"),
                ));
            }
            meta.kind = FileKind::from_stat(fields[0]);
            meta.len = fields[1].trim().parse().unwrap_or(0);
            meta.permissions =
                Permissions::from_mode(u32::from_str_radix(fields[2].trim(), 8).unwrap_or(0));
            meta.uid = fields[3].trim().parse().unwrap_or(0);
            meta.gid = fields[4].trim().parse().unwrap_or(0);
            meta.modified = fields[5]
                .trim()
                .parse::<u64>()
                .ok()
                .map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs));
            meta.inode = fields
                .get(6)
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
            found = true;
        }

        if !found {
            // Coarse fallback when a host has no usable `stat`.
            meta.kind = match parse::marker(stdout, "TYPE") {
                Some(value) => FileKind::from_type_marker(value),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "failed to stat: unrecognized probe output",
                    ));
                }
            };
        }

        // `stat` follows symlinks only with `-L`; a link observed without
        // dereferencing is reported as a link even though `stat` printed the
        // link's own type (which already is "symbolic link", but BusyBox is
        // not always explicit).
        if !deref && parse::flag(stdout, "SYMLINK", "yes") {
            meta.kind = FileKind::Symlink;
        }

        Ok(Some(meta))
    }

    /// The file type.
    pub fn file_type(&self) -> FileType {
        FileType::new(self.kind)
    }

    /// Whether this is a regular file.
    pub fn is_file(&self) -> bool {
        self.kind == FileKind::File
    }

    /// Whether this is a directory.
    pub fn is_dir(&self) -> bool {
        self.kind == FileKind::Dir
    }

    /// Whether this is a symbolic link.
    pub fn is_symlink(&self) -> bool {
        self.kind == FileKind::Symlink
    }

    /// Size in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the file is empty (size 0).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// POSIX permissions.
    pub fn permissions(&self) -> Permissions {
        self.permissions
    }

    /// Owner user id.
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Owner group id.
    pub fn gid(&self) -> u32 {
        self.gid
    }

    /// Last modification time, when the host reported one.
    pub fn modified(&self) -> io::Result<SystemTime> {
        self.modified.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "remote host did not report a modification time",
            )
        })
    }

    /// Inode number (0 when not reported).
    pub fn inode(&self) -> u64 {
        self.inode
    }
}

/// An entry returned by [`Fs::read_dir`].
///
/// `file_type()` and `metadata()` are `async` because the type is not known
/// until the remote host is asked — unlike [`std::fs::DirEntry`], whose type
/// comes from the directory listing the OS already returned.
#[derive(Clone)]
pub struct DirEntry {
    fs: Fs,
    path: PathBuf,
    name: OsString,
}

impl std::fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirEntry")
            .field("path", &self.path)
            .finish()
    }
}

impl DirEntry {
    pub(crate) fn new(fs: Fs, path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        Self { fs, path, name }
    }

    /// Full path of the entry, as produced by the remote listing.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Final component of the path.
    pub fn file_name(&self) -> &OsStr {
        &self.name
    }

    /// Fetch the entry's metadata without following symlinks, like
    /// [`std::fs::DirEntry::metadata`].
    pub async fn metadata(&self) -> io::Result<Metadata> {
        self.fs.symlink_metadata(&self.path).await
    }

    /// Fetch the entry's file type without following symlinks, like
    /// [`std::fs::DirEntry::file_type`].
    pub async fn file_type(&self) -> io::Result<FileType> {
        Ok(self.metadata().await?.file_type())
    }
}

/// Iterator over the entries of a remote directory, like [`std::fs::ReadDir`].
///
/// Entries are fetched eagerly with a single command (one round trip); the
/// per-entry type/metadata lookups are lazy.
pub struct ReadDir {
    inner: std::vec::IntoIter<DirEntry>,
}

impl std::fmt::Debug for ReadDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadDir")
            .field("remaining", &self.inner.len())
            .finish()
    }
}

impl ReadDir {
    pub(crate) fn new(entries: Vec<DirEntry>) -> Self {
        Self {
            inner: entries.into_iter(),
        }
    }
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(Ok)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

/// Free/total space of a remote filesystem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskSpace {
    /// Total size of the filesystem in bytes.
    pub total_bytes: u64,
    /// Space available to the current user in bytes.
    pub available_bytes: u64,
    /// Whether the figures are a real measurement.
    ///
    /// `false` means the host could not report space (no `stat -f`, no `df`,
    /// unparsable output), which is different from a genuine `0` on a full
    /// filesystem.
    pub known: bool,
    /// Path the figures were taken from.
    ///
    /// Equals the queried path when it exists; otherwise it is the nearest
    /// existing ancestor (often `/`), i.e. the filesystem that would hold a
    /// newly created directory.
    pub probe_path: PathBuf,
}

/// Rich status of a directory, as returned by [`Fs::dir_status`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirStatus {
    /// The directory that was inspected.
    pub path: PathBuf,
    /// Whether it exists.
    pub exists: bool,
    /// Whether it exists and is writable by the current user.
    pub writable: bool,
    /// The directory does not exist but its nearest existing ancestor is
    /// writable, so it can be created.
    pub creatable: bool,
    /// Free/total space of the filesystem that holds it.
    pub space: DiskSpace,
    /// Why the directory is unusable, when it is.
    pub error: Option<String>,
}

impl DirStatus {
    /// Whether files can be written to the directory (creating it if needed).
    pub fn usable(&self) -> bool {
        self.writable || (!self.exists && self.creatable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gnu_stat_line() {
        let out = "STAT=regular file|1234|644|1000|1000|1700000000|42\n";
        let meta = Metadata::parse(out, true).unwrap().unwrap();
        assert_eq!(meta.file_type().kind(), FileKind::File);
        assert_eq!(meta.len(), 1234);
        assert_eq!(meta.permissions().mode(), 0o644);
        assert_eq!(meta.uid(), 1000);
        assert_eq!(meta.gid(), 1000);
        assert_eq!(
            meta.modified().unwrap(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000)
        );
        assert_eq!(meta.inode(), 42);
        assert!(!meta.permissions().readonly());
    }

    #[test]
    fn parses_bsd_stat_line() {
        let out = "STAT=Directory|4096|755|501|20|1700000000|7\n";
        let meta = Metadata::parse(out, true).unwrap().unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.permissions().mode(), 0o755);
    }

    #[test]
    fn parses_symlink() {
        let out = "SYMLINK=yes\nTARGET=/tmp/target\nSTAT=symbolic link|10|777|0|0|1700000000|3\n";
        let meta = Metadata::parse(out, false).unwrap().unwrap();
        assert!(meta.is_symlink());

        // Dereferencing reports the target's type.
        let out = "STAT=regular file|10|644|0|0|1700000000|3\n";
        let meta = Metadata::parse(out, true).unwrap().unwrap();
        assert!(meta.is_file());
    }

    #[test]
    fn missing_path_is_none() {
        assert!(Metadata::parse("NOTFOUND=1\n", true).unwrap().is_none());
    }

    #[test]
    fn uses_type_fallback() {
        let meta = Metadata::parse("TYPE=dir\n", false).unwrap().unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.len(), 0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Metadata::parse("hello\n", false).is_err());
    }

    #[test]
    fn classifies_stat_types() {
        assert_eq!(FileKind::from_stat("regular empty file"), FileKind::File);
        assert_eq!(FileKind::from_stat("Directory"), FileKind::Dir);
        assert_eq!(FileKind::from_stat("symbolic link"), FileKind::Symlink);
        assert_eq!(
            FileKind::from_stat("character special file"),
            FileKind::CharDevice
        );
        assert_eq!(
            FileKind::from_stat("block special file"),
            FileKind::BlockDevice
        );
        assert_eq!(FileKind::from_stat("fifo"), FileKind::Fifo);
        assert_eq!(FileKind::from_stat("socket"), FileKind::Socket);
        assert_eq!(FileKind::from_stat("weird"), FileKind::Other);
    }

    #[test]
    fn permissions_readonly() {
        assert!(Permissions::from_mode(0o444).readonly());
        assert!(!Permissions::from_mode(0o644).readonly());
        assert_eq!(Permissions::from_mode(0o7777).mode(), 0o7777);
    }
}
