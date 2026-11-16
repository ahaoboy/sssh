//! Input types + helpers for assembling a [`Config`](crate::Config) from
//! user-facing values.

use std::path::Path;

use clap::ValueEnum;

use crate::{Auth, Config, Error, parse_target};

/// Operating mode for the SSH client (the CLI `-m` values).
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Interactive PTY shell (default).
    Shell,
    /// Authenticate and keep the connection idle until interrupted.
    Idle,
    /// Execute a command non-interactively and print its output.
    Cmd,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Shell => "shell",
            Self::Idle => "idle",
            Self::Cmd => "cmd",
        })
    }
}

/// The set of inputs a CLI-style caller provides.
///
/// This is the parsed, framework-agnostic form of the `sssh` command line:
/// clap fills it in for the built-in binary, but any embedder can construct
/// one directly and call [`run_shell`](super::run_shell) /
/// [`run_cmd`](super::run_cmd) — or build the [`Config`]
/// themselves via [`build_config`].
pub struct Input {
    pub target: String,
    pub port: u16,
    pub identity: Option<String>,
    pub command: Option<String>,
    pub shell: Option<String>,
    pub mode: Mode,
    /// Trailing positional command (`sssh host command`), used when `-c` is
    /// absent.
    pub trailing_cmd: Option<String>,
}

/// Quote a string for safe inclusion as a single shell argument (POSIX-style
/// single-quote wrapping: `'` → `'\''`).
///
/// Re-exported from the library-level shell helper so the CLI and the
/// filesystem API share one implementation.
pub use crate::shell::shell_quote;

/// Reconcile the remote command from `command` (or `trailing_cmd`) and an
/// optional `shell`.
///
/// - `command` + `shell` → `SHELL -c 'CMD'` (runs inside that shell).
/// - `command` without `shell` → `CMD` verbatim.
/// - no `command` + `shell` → `SHELL` (start it as the interactive shell).
/// - neither → `None` (use the remote default login shell).
pub fn resolve_command(
    command: Option<&str>,
    trailing: Option<&str>,
    shell: Option<&str>,
) -> Option<String> {
    match (command.or(trailing), shell) {
        (Some(cmd), Some(sh)) => Some(format!("{} -c {}", sh, shell_quote(cmd))),
        (Some(cmd), None) => Some(cmd.to_string()),
        (None, Some(sh)) => Some(sh.to_string()),
        (None, None) => None,
    }
}

/// Load credentials from a key file, an explicit password, `SSSH_PASSWORD`,
/// or `ask` (in that order).
pub fn credentials(
    identity: Option<&Path>,
    password: Option<&str>,
    ask: impl Fn(&str) -> std::io::Result<String>,
) -> Result<Auth, Error> {
    if let Some(path) = identity {
        let key = std::fs::read_to_string(path).map_err(|e| {
            Error::Other(format!("failed to read SSH key '{}': {e}", path.display()))
        })?;
        return Ok(Auth::key(key));
    }
    if let Some(password) = password.filter(|value| !value.is_empty()) {
        return Ok(Auth::password(password));
    }
    match std::env::var("SSSH_PASSWORD") {
        Ok(password) if !password.is_empty() => Ok(Auth::password(password)),
        _ => Ok(Auth::password(ask("Password: ")?)),
    }
}

/// Build a ready-to-use [`Config`] from [`Input`], resolving auth
/// (key file vs. password env/prompt) and terminal size.
///
/// `ask` is invoked when no key and no `SSSH_PASSWORD` env var is available;
/// pass `|prompt| rpassword::prompt_password(prompt)` for an interactive TTY
/// prompt, or a custom closure for non-interactive flows.
pub fn build_config(
    input: &Input,
    ask: impl Fn(&str) -> std::io::Result<String>,
) -> Result<Config, Error> {
    let (username, host) = parse_target(&input.target).map_err(Error::Other)?;

    // `cmd` mode captures stdout — keep the prompt off it.
    let ask = |prompt: &str| {
        if input.mode == Mode::Cmd {
            use std::io::Write;
            eprint!("{prompt}");
            let _ = std::io::stderr().flush();
            ask("")
        } else {
            ask(prompt)
        }
    };
    let (password, private_key) =
        credentials(input.identity.as_deref().map(Path::new), None, ask)?.into();

    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

    Ok(Config {
        host,
        port: input.port,
        username,
        password,
        private_key,
        command: resolve_command(
            input.command.as_deref(),
            input.trailing_cmd.as_deref(),
            input.shell.as_deref(),
        ),
        cols: cols as u32,
        rows: rows as u32,
        timeout_secs: None,
        host_key_verifier: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_commands_inside_the_requested_shell() {
        assert_eq!(
            resolve_command(Some("ls -l"), None, Some("/bin/fish")).as_deref(),
            Some("/bin/fish -c 'ls -l'")
        );
        assert_eq!(
            resolve_command(None, Some("uname"), None).as_deref(),
            Some("uname")
        );
        assert_eq!(
            resolve_command(None, None, Some("/bin/fish")).as_deref(),
            Some("/bin/fish")
        );
        assert_eq!(resolve_command(None, None, None), None);
    }

    #[test]
    fn credentials_prefer_keys_and_reject_empty_passwords() {
        let key = std::env::temp_dir().join("sssh-test-key");
        std::fs::write(&key, "KEY").unwrap();
        match credentials(Some(&key), Some("ignored"), |_| unreachable!()).unwrap() {
            Auth::PrivateKey(key) => assert_eq!(key, "KEY"),
            _ => panic!("expected a private key"),
        }
        let _ = std::fs::remove_file(key);

        match credentials(None, Some("pass"), |_| unreachable!()).unwrap() {
            Auth::Password(pass) => assert_eq!(pass, "pass"),
            _ => panic!("expected a password"),
        }
    }
}
