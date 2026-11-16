//! `sssh fs …` — command-line access to the remote filesystem API.
//!
//! Every subcommand is a thin wrapper around [`crate::fs::Fs`]:
//!
//! ```text
//! sssh fs read   user@host ~/.bashrc
//! sssh fs write  user@host ~/.config/app.json --from ./app.json --mkdirs
//! sssh fs ls     user@host /etc --long
//! sssh fs mkdir  user@host /tmp/work -p
//! sssh fs rm     user@host /tmp/work -r
//! sssh fs stat   user@host /etc/hosts
//! sssh fs df     user@host /
//! sssh fs chmod  user@host 755 ~/tool
//! ```
//!
//! Connection flags (`-p/--port`, `-i/--identity`, `--password`, `--timeout`)
//! are accepted once, before the operation name. The password may also come
//! from `SSSH_PASSWORD` or an interactive prompt.

mod inspect;
mod read;
mod write;

use clap::{Args, Subcommand};
use std::path::PathBuf;

use crate::fs::{FileKind, Metadata};
use crate::{Client, Error};

use inspect as ops_inspect;
use read as ops_read;
use write as ops_write;

/// Arguments for `sssh fs`.
#[derive(Debug, Args)]
pub struct FsArgs {
    #[command(flatten)]
    pub conn: ConnArgs,
    #[command(subcommand)]
    pub command: FsCommand,
}

/// Connection options shared by every filesystem subcommand.
#[derive(Debug, Args)]
pub struct ConnArgs {
    /// SSH port.
    #[arg(short, long, default_value_t = 22, value_name = "PORT")]
    pub port: u16,

    /// Private key file.
    #[arg(short = 'i', long, value_name = "FILE")]
    pub identity: Option<PathBuf>,

    /// Password (prefer SSSH_PASSWORD or the interactive prompt).
    #[arg(long, value_name = "PASSWORD")]
    pub password: Option<String>,

    /// Connection timeout in seconds.
    #[arg(long, default_value_t = 30, value_name = "SECS")]
    pub timeout: u64,

    /// Verbose logging.
    #[arg(short, long)]
    pub verbose: bool,
}

impl ConnArgs {
    async fn connect(&self, target: &str) -> Result<Client, Error> {
        let auth = crate::cli::credentials(
            self.identity.as_deref(),
            self.password.as_deref(),
            |prompt| rpassword::prompt_password(prompt),
        )?;
        Client::connect_target(target, self.port, auth, Some(self.timeout.max(1))).await
    }
}

/// Filesystem operations.
#[derive(Debug, Subcommand)]
pub enum FsCommand {
    /// Print a remote file (raw bytes, or --text for UTF-8).
    Read(ReadArgs),
    /// Write a remote file from stdin, --content, or --from FILE.
    Write(WriteArgs),
    /// List a remote directory.
    Ls(LsArgs),
    /// Create a remote directory.
    Mkdir(MkdirArgs),
    /// Remove remote files or directories.
    Rm(RmArgs),
    /// Move/rename a remote path.
    Mv(MvArgs),
    /// Copy a remote file.
    Cp(CpArgs),
    /// Print metadata for remote paths.
    Stat(StatArgs),
    /// Change remote permission bits.
    Chmod(ChmodArgs),
    /// Show directory status and free space.
    Df(DfArgs),
    /// Exit 0 when the path exists, 1 otherwise.
    Exists(PathArg),
    /// Print a symbolic link's target.
    Readlink(PathArg),
    /// Print the canonical path.
    Canonicalize(PathArg),
    /// Create a symbolic link.
    Symlink(SymlinkArgs),
    /// Create a hard link.
    Hardlink(SymlinkArgs),
    /// Create an empty file (touch).
    Touch(TouchArgs),
}

impl FsCommand {
    fn target(&self) -> &str {
        match self {
            Self::Read(args) => &args.target,
            Self::Write(args) => &args.target,
            Self::Ls(args) => &args.target,
            Self::Mkdir(args) => &args.target,
            Self::Rm(args) => &args.target,
            Self::Mv(args) => &args.target,
            Self::Cp(args) => &args.target,
            Self::Stat(args) => &args.target,
            Self::Chmod(args) => &args.target,
            Self::Df(args) => &args.target,
            Self::Exists(args) => &args.target,
            Self::Readlink(args) => &args.target,
            Self::Canonicalize(args) => &args.target,
            Self::Symlink(args) => &args.target,
            Self::Hardlink(args) => &args.target,
            Self::Touch(args) => &args.target,
        }
    }
}

/// `target` + a single remote path.
#[derive(Debug, Args)]
pub struct PathArg {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote path.
    pub path: String,
}

#[derive(Debug, Args)]
pub struct ReadArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote file path.
    pub path: String,
    /// Save to this local file instead of stdout.
    #[arg(short = 'o', long, value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// Require valid UTF-8 and print as text.
    #[arg(long)]
    pub text: bool,
}

#[derive(Debug, Args)]
pub struct WriteArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote file path.
    pub path: String,
    /// Literal contents (default: read stdin).
    #[arg(short = 'c', long, value_name = "TEXT", conflicts_with = "from")]
    pub content: Option<String>,
    /// Read contents from a local file.
    #[arg(long, value_name = "FILE")]
    pub from: Option<PathBuf>,
    /// POSIX permission bits (octal, e.g. 755).
    #[arg(long, value_name = "MODE")]
    pub mode: Option<String>,
    /// Append instead of truncating.
    #[arg(short = 'a', long)]
    pub append: bool,
    /// Create missing parent directories first.
    #[arg(long)]
    pub mkdirs: bool,
}

#[derive(Debug, Args)]
pub struct LsArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote directory (default: ~).
    #[arg(default_value = "~")]
    pub path: String,
    /// Long format: type, mode, size, mtime.
    #[arg(short, long)]
    pub long: bool,
    /// Print full remote paths instead of names.
    #[arg(long)]
    pub paths: bool,
}

#[derive(Debug, Args)]
pub struct MkdirArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote directory.
    pub path: String,
    /// Create parents as needed (like mkdir -p).
    #[arg(short, long)]
    pub parents: bool,
}

#[derive(Debug, Args)]
pub struct RmArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote paths.
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<String>,
    /// Remove directories recursively.
    #[arg(short, long, conflicts_with = "dir")]
    pub recursive: bool,
    /// Remove empty directories.
    #[arg(short, long)]
    pub dir: bool,
    /// Ignore missing paths.
    #[arg(short, long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct MvArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Source path.
    pub from: String,
    /// Destination path.
    pub to: String,
}

#[derive(Debug, Args)]
pub struct CpArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Source file.
    pub from: String,
    /// Destination file.
    pub to: String,
}

#[derive(Debug, Args)]
pub struct StatArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote paths.
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ChmodArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Octal mode, e.g. 755 or 0644.
    pub mode: String,
    /// Remote paths.
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DfArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote path (default: ~).
    #[arg(default_value = "~")]
    pub path: String,
}

#[derive(Debug, Args)]
pub struct SymlinkArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Link target (may be relative).
    pub original: String,
    /// Remote link path.
    pub link: String,
}

#[derive(Debug, Args)]
pub struct TouchArgs {
    /// Connection target: `[user@]host`.
    pub target: String,
    /// Remote paths.
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<String>,
}

/// Execute a parsed `sssh fs …` invocation.
pub async fn run(args: FsArgs) -> Result<(), Error> {
    let target = args.command.target().to_string();
    let client = args.conn.connect(&target).await?;
    let fs = client.fs();

    match &args.command {
        FsCommand::Read(args) => ops_read::read(&fs, args).await,
        FsCommand::Write(args) => ops_write::write(&fs, args).await,
        FsCommand::Ls(args) => ops_read::ls(&fs, args).await,
        FsCommand::Mkdir(args) => ops_write::mkdir(&fs, args).await,
        FsCommand::Rm(args) => ops_write::rm(&fs, args).await,
        FsCommand::Mv(args) => ops_write::mv(&fs, args).await,
        FsCommand::Cp(args) => ops_write::cp(&fs, args).await,
        FsCommand::Stat(args) => ops_inspect::stat(&fs, args).await,
        FsCommand::Chmod(args) => ops_write::chmod(&fs, args).await,
        FsCommand::Df(args) => ops_inspect::df(&fs, args).await,
        FsCommand::Exists(args) => ops_inspect::exists(&fs, args).await,
        FsCommand::Readlink(args) => ops_inspect::readlink(&fs, args).await,
        FsCommand::Canonicalize(args) => ops_inspect::canonicalize(&fs, args).await,
        FsCommand::Symlink(args) => ops_write::symlink(&fs, args).await,
        FsCommand::Hardlink(args) => ops_write::hard_link(&fs, args).await,
        FsCommand::Touch(args) => ops_write::touch(&fs, args).await,
    }
}

// ── Shared formatting / parsing helpers ─────────────────────────────────

/// Parse an octal permission string (`755`, `0755`, `0o755`).
pub(super) fn parse_mode(value: &str) -> Result<u32, Error> {
    let digits = value
        .strip_prefix("0o")
        .or_else(|| value.strip_prefix("0O"))
        .unwrap_or(value);
    let mode = u32::from_str_radix(digits, 8)
        .map_err(|_| Error::Other(format!("invalid octal mode '{value}'")))?;
    if mode > 0o7777 {
        return Err(Error::Other(format!(
            "invalid mode '{value}': must be between 0 and 7777"
        )));
    }
    Ok(mode)
}

/// Human readable byte size.
pub(super) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `rwxr-xr-x`-style permission string.
pub(super) fn mode_string(mode: u32) -> String {
    let mut out = String::with_capacity(9);
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        out.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        out.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    out
}

/// `ls -l`-style leading type character.
pub(super) fn kind_char(kind: FileKind) -> char {
    match kind {
        FileKind::File => '-',
        FileKind::Dir => 'd',
        FileKind::Symlink => 'l',
        FileKind::Fifo => 'p',
        FileKind::Socket => 's',
        FileKind::CharDevice => 'c',
        FileKind::BlockDevice => 'b',
        FileKind::Other => '?',
    }
}

/// Human readable kind name.
pub(super) fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::File => "file",
        FileKind::Dir => "dir",
        FileKind::Symlink => "symlink",
        FileKind::Fifo => "fifo",
        FileKind::Socket => "socket",
        FileKind::CharDevice => "char device",
        FileKind::BlockDevice => "block device",
        FileKind::Other => "other",
    }
}

/// Print one `stat`-style block.
pub(super) fn print_metadata(path: &str, meta: &Metadata) {
    println!("path: {path}");
    println!("type: {}", kind_name(meta.file_type().kind()));
    println!("size: {} ({})", meta.len(), format_bytes(meta.len()));
    println!(
        "mode: {:04o} ({})",
        meta.permissions().mode(),
        mode_string(meta.permissions().mode())
    );
    println!("uid: {}", meta.uid());
    println!("gid: {}", meta.gid());
    match meta.modified() {
        Ok(time) => {
            let secs = time
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            println!("modified: {secs}");
        }
        Err(_) => println!("modified: (unknown)"),
    }
    println!("inode: {}", meta.inode());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_octal_modes() {
        assert_eq!(parse_mode("755").unwrap(), 0o755);
        assert_eq!(parse_mode("0755").unwrap(), 0o755);
        assert_eq!(parse_mode("0o644").unwrap(), 0o644);
        assert!(parse_mode("999").is_err());
        assert!(parse_mode("rwx").is_err());
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
    }

    #[test]
    fn formats_modes() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o644), "rw-r--r--");
        assert_eq!(mode_string(0o000), "---------");
    }

    #[test]
    fn maps_kinds() {
        assert_eq!(kind_char(FileKind::Dir), 'd');
        assert_eq!(kind_char(FileKind::File), '-');
        assert_eq!(kind_char(FileKind::Symlink), 'l');
        assert_eq!(kind_name(FileKind::BlockDevice), "block device");
    }
}
