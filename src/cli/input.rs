//! Input types + helpers for assembling a [`Config`](crate::Config) from
//! user-facing values.

use crate::Config;

/// Operating mode for the SSH client.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Interactive PTY shell (default).
    Shell,
    /// Authenticate and keep the connection idle until interrupted.
    Idle,
    /// Execute a command non-interactively and print its output.
    Cmd,
}

/// The set of inputs a CLI-style caller provides.
///
/// This is the parsed, framework-agnostic form of the `sssh` command line:
/// clap fills it in for the built-in binary, but any embedder can construct
/// one directly and call [`run`](super::run) — or build the [`Config`]
/// themselves via [`build_config`].
pub struct CliInput {
    pub target: String,
    pub port: u16,
    pub identity: Option<String>,
    pub command: Option<String>,
    pub shell: Option<String>,
    pub mode: Mode,
    pub verbose: bool,
    /// Trailing positional command (`sssh host command`), used when `-c` is
    /// absent.
    pub trailing_cmd: Option<String>,
}

/// Quote a string for safe inclusion as a single shell argument (POSIX-style
/// single-quote wrapping: `'` → `'\''`).
pub fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Reconcile the remote command from `command` (or `trailing_cmd`) and an
/// optional `shell`.
///
/// - `command` + `shell` → `SHELL -c 'CMD'` (runs inside that shell).
/// - `command` without `shell` → `CMD` verbatim.
/// - no `command` + `shell` → `SHELL` (start it as the interactive shell).
/// - neither → `None` (use the remote default login shell).
pub fn reconcile_command(
    command: Option<&str>,
    trailing: Option<&str>,
    shell: Option<&str>,
) -> Option<String> {
    let raw = command.or(trailing);
    // Match first, clone only when needed.
    match (raw, shell) {
        (Some(cmd), Some(sh)) => Some(format!("{} -c {}", sh, shell_quote(cmd))),
        (Some(cmd), None) => Some(cmd.to_string()),
        (None, Some(sh)) => Some(sh.to_string()),
        (None, None) => None,
    }
}

/// Build a ready-to-use [`Config`] from a [`CliInput`], resolving auth
/// (key file vs. password env/prompt) and terminal size.
///
/// `password_prompt` is invoked when no key and no `SSSH_PASSWORD` env var
/// is available; pass `&rpassword::prompt_password` for an interactive TTY
/// prompt, or a custom closure for non-interactive flows.
pub fn build_config(
    input: &CliInput,
    password_prompt: impl Fn(&str) -> std::io::Result<String>,
) -> Result<Config, Box<dyn std::error::Error>> {
    let (username, host) = crate::parse_target(&input.target).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
    })?;

    let private_key = match input.identity.as_ref() {
        Some(path) => Some(std::fs::read_to_string(path)?),
        None => None,
    };

    let password = if private_key.is_none() {
        match std::env::var("SSSH_PASSWORD") {
            Ok(p) if !p.is_empty() => Some(p),
            _ => {
                // `cmd` mode captures stdout — avoid polluting it with a
                // TTY prompt; print the prompt to stderr and read blind.
                if input.mode == Mode::Cmd {
                    use std::io::Write;
                    eprint!("Password: ");
                    let _ = std::io::stderr().flush();
                    Some(password_prompt("")?)
                } else {
                    Some(password_prompt("Password: ")?)
                }
            }
        }
    } else {
        None
    };

    let command = reconcile_command(
        input.command.as_deref(),
        input.trailing_cmd.as_deref(),
        input.shell.as_deref(),
    );

    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

    Ok(Config {
        host,
        port: input.port,
        username,
        password,
        private_key,
        command,
        cols: cols as u32,
        rows: rows as u32,
        timeout_secs: None,
        host_key_verifier: None,
    })
}
