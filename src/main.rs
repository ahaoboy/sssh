//! `sssh` — a simple SSH client CLI.
//!
//! Connects to a remote host and supports three modes:
//! - **shell** (default): interactive PTY shell (or an interactive command).
//! - **idle**: authenticate and keep the connection alive until interrupted.
//! - **cmd**: execute a command non-interactively and print its captured
//!   output, exiting with the remote process's exit code.
//!
//! `sssh fs …` provides command-line access to the remote filesystem API
//! ([`sssh::fs`]): read, write, ls, mkdir, rm, mv, cp, stat, chmod, df, …
//!
//! All execution logic lives in the [`sssh`] library; this binary only
//! parses arguments, sets up logging, and dispatches. The `cmd` mode uses
//! [`Session::exec_stream`] directly — the library's generic primitives
//! are designed to be composable by any caller.

use clap::CommandFactory;
use clap::Parser;
use clap::Subcommand;
use sssh::Error;
use sssh::cli::fs::FsArgs;
use sssh::cli::{Input, Mode, build_config, resolve_command, run_cmd, run_idle, run_shell};
use std::path::PathBuf;
use tracing::info;

/// A simple SSH client.
#[derive(Parser, Debug)]
#[command(name = "sssh", version, about = "A simple SSH client")]
struct Args {
    /// Remote filesystem commands (`sssh fs --help`).
    #[command(subcommand)]
    subcommand: Option<Command>,

    /// Connection target: [user@]host
    #[arg(value_name = "TARGET")]
    target: Option<String>,

    /// SSH port
    #[arg(short, long, default_value_t = 22)]
    port: u16,

    /// Private key file
    #[arg(short = 'i', long, value_name = "FILE")]
    identity: Option<PathBuf>,

    /// Remote command to execute.
    ///
    /// Given via `-c` or as a trailing positional (`sssh host command`).
    /// In `shell` mode it is run inside the PTY; in `cmd` mode it is run
    /// non-interactively and its captured output + exit code are returned.
    #[arg(short, long)]
    command: Option<String>,

    /// Shell to run the command in (e.g. `/jffs/fish`).
    ///
    /// When given together with `-c CMD`, the remote command becomes
    /// `SHELL -c 'CMD'` so it executes inside that shell (useful when the
    /// command is only findable in that shell's PATH or is a shell function).
    /// Without `-c`, starts SHELL as the interactive shell instead of the
    /// default login shell (like `ssh -t host /jffs/fish`).
    #[arg(short = 't', long = "shell", value_name = "SHELL")]
    shell: Option<String>,

    /// Operating mode.
    #[arg(short = 'm', long, value_enum, default_value_t = Mode::Shell)]
    mode: Mode,

    /// Verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Remote command (trailing positional, like `ssh host command`).
    #[arg(value_name = "REMOTE_CMD")]
    cmd: Option<String>,
}

/// Subcommands that do not open an interactive session.
#[derive(Subcommand, Debug)]
enum Command {
    /// Remote filesystem operations (read, write, ls, mkdir, rm, stat, df, …).
    Fs(FsArgs),
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut cli = Args::parse();

    if cli.target.is_none() && cli.subcommand.is_none() {
        Args::command()
            .error(
                clap::error::ErrorKind::MissingRequiredArgument,
                "a connection target ([user@]host) or a subcommand is required",
            )
            .exit();
    }

    let verbose = cli.verbose
        || match &cli.subcommand {
            Some(Command::Fs(fs)) => fs.conn.verbose,
            None => false,
        };

    // Tracing
    let default_filter = if verbose { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .init();

    // `sssh fs …` runs against the remote filesystem API.
    if let Some(command) = cli.subcommand.take() {
        match command {
            Command::Fs(fs) => return sssh::cli::fs::run(fs).await,
        }
    }

    let target = cli
        .target
        .expect("clap requires a target unless a subcommand is used");
    let mode = cli.mode;
    let input = Input {
        target,
        port: cli.port,
        identity: cli
            .identity
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        command: cli.command.clone(),
        shell: cli.shell.clone(),
        mode,
        trailing_cmd: cli.cmd.clone(),
    };

    let command = resolve_command(
        input.command.as_deref(),
        input.trailing_cmd.as_deref(),
        input.shell.as_deref(),
    );

    info!(
        "Connecting to {} (mode={:?}, shell={:?}, cmd={:?})",
        input.target, input.mode, input.shell, command,
    );

    let config = build_config(&input, |prompt| rpassword::prompt_password(prompt))?;

    match mode {
        Mode::Shell => {
            run_shell(config).await?;
            Ok(())
        }
        Mode::Idle => {
            run_idle(config).await?;
            Ok(())
        }
        Mode::Cmd => {
            let command = command
                .as_ref()
                .ok_or_else(|| Error::Other("cmd mode requires a command".into()))?;
            run_cmd(config, command, tokio::io::stdout(), tokio::io::stderr()).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_target_and_command() {
        let args = Args::try_parse_from(["sssh", "user@host", "-c", "ls"]).unwrap();
        assert!(args.subcommand.is_none());
        assert_eq!(args.target.as_deref(), Some("user@host"));
        assert_eq!(args.command.as_deref(), Some("ls"));
    }

    #[test]
    fn parses_trailing_command() {
        let args = Args::try_parse_from(["sssh", "user@host", "ls -la"]).unwrap();
        assert_eq!(args.cmd.as_deref(), Some("ls -la"));
    }

    #[test]
    fn parses_fs_read() {
        let args = Args::try_parse_from(["sssh", "fs", "read", "user@host", "~/x"]).unwrap();
        match args.subcommand.unwrap() {
            Command::Fs(fs) => match fs.command {
                sssh::cli::fs::FsCommand::Read(read) => {
                    assert_eq!(read.target, "user@host");
                    assert_eq!(read.path, "~/x");
                }
                _ => panic!("expected read"),
            },
        }
    }

    #[test]
    fn parses_fs_connection_flags() {
        let args = Args::try_parse_from(["sssh", "fs", "-p", "2222", "stat", "u@h", "/x"]).unwrap();
        match args.subcommand.unwrap() {
            Command::Fs(fs) => assert_eq!(fs.conn.port, 2222),
        }
    }

    #[test]
    fn empty_invocation_has_no_target_or_subcommand() {
        let args = Args::try_parse_from(["sssh"]).unwrap();
        assert!(args.target.is_none());
        assert!(args.subcommand.is_none());
    }
}
