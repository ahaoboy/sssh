//! `sssh` — a simple SSH client CLI.
//!
//! Connects to a remote host and supports three modes:
//! - **shell** (default): interactive PTY shell (or an interactive command).
//! - **idle**: authenticate and keep the connection alive until interrupted.
//! - **cmd**: execute a command non-interactively and print its captured
//!   output, exiting with the remote process's exit code.
//!
//! All execution logic lives in the [`sssh`] library; this binary only
//! parses arguments, sets up logging, and dispatches. The `cmd` mode uses
//! [`Session::exec_stream`] directly — the library's generic primitives
//! are designed to be composable by any caller.

use clap::Parser;
use clap::ValueEnum;
use sssh::Error;
use sssh::cli::{CliInput, Mode, build_config, reconcile_command, run_cmd, run_idle, run_shell};
use std::path::PathBuf;
use tracing::info;

/// A simple SSH client.
#[derive(Parser, Debug)]
#[command(name = "sssh", version, about = "A simple SSH client")]
struct Args {
    /// Connection target: [user@]host
    #[arg(value_name = "TARGET")]
    target: String,

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
    #[arg(short = 'm', long, value_enum, default_value_t = ModeArg::Shell)]
    mode: ModeArg,

    /// Verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Remote command (trailing positional, like `ssh host command`).
    #[arg(value_name = "COMMAND")]
    cmd: Option<String>,
}

/// Clap-facing mirror of [`Mode`].
#[derive(Copy, Clone, Debug, ValueEnum)]
enum ModeArg {
    /// Interactive PTY shell (default).
    Shell,
    /// Authenticate and keep the connection idle until interrupted.
    Idle,
    /// Execute a command non-interactively and print its output.
    Cmd,
}

impl From<ModeArg> for Mode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Shell => Mode::Shell,
            ModeArg::Idle => Mode::Idle,
            ModeArg::Cmd => Mode::Cmd,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let cli = Args::parse();

    // Tracing
    let default_filter = if cli.verbose { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .init();

    let mode: Mode = cli.mode.into();
    let input = CliInput {
        target: cli.target.clone(),
        port: cli.port,
        identity: cli
            .identity
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        command: cli.command.clone(),
        shell: cli.shell.clone(),
        mode,
        verbose: cli.verbose,
        trailing_cmd: cli.cmd.clone(),
    };

    let command = reconcile_command(
        input.command.as_deref(),
        input.trailing_cmd.as_deref(),
        input.shell.as_deref(),
    );

    info!(
        "Connecting to {} (mode={:?}, shell={:?}, cmd={:?})",
        input.target, input.mode, input.shell, command,
    );

    let config = build_config(&input, |p| rpassword::prompt_password(p))
        .map_err(|e| Error::Other(e.to_string()))?;

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
