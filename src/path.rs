//! `PATH` registration for a remote shell.
//!
//! A remote host has no library to call, so the config file is edited with a
//! short POSIX command. Appending is guarded by `grep -qxF`, which makes the
//! operation idempotent when the same install runs twice.

use crate::shell::shell_quote;

/// Command that appends `line` to `$HOME/file` exactly once.
///
/// `line` is inserted both as the `line=…` assignment and as the `grep`
/// pattern, so it must already be quoted for the shell; the caller builds it
/// from [`which_shell::Shell::path_line`].
pub fn add_line_command(file: &str, line: &str) -> String {
    let line = shell_quote(line);
    format!(
        "rc=\"$HOME/{file}\"; mkdir -p \"$(dirname \"$rc\")\"; \
         line={line}; grep -qxF \"$line\" \"$rc\" 2>/dev/null || printf '\\n%s\\n' \"$line\" >> \"$rc\""
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command must create the config directory, stay idempotent, and pass
    /// the line through quoting instead of interpolating it.
    #[test]
    fn add_line_command_is_idempotent_and_quoted() {
        let command = add_line_command(".bashrc", r#"export PATH="$PATH:/opt/we$ird""#);

        assert!(command.contains("rc=\"$HOME/.bashrc\""), "{command}");
        assert!(
            command.contains("mkdir -p \"$(dirname \"$rc\")\""),
            "{command}"
        );
        assert!(command.contains("grep -qxF \"$line\""), "{command}");
        assert!(
            command.contains(r#"line='export PATH="$PATH:/opt/we$ird"'"#),
            "{command}"
        );
    }
}
