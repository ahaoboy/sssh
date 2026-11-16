//! Captured command output, mirroring [`std::process::Output`].
//!
//! `stdout`/`stderr` are raw bytes so binary payloads (file reads, `tar`
//! streams, …) survive unchanged; use the `*_lossy` helpers for text.

/// Exit status of a remote command, like [`std::process::ExitStatus`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ExitStatus {
    code: Option<i32>,
    signal: Option<String>,
}

impl ExitStatus {
    /// The command exited with `code`.
    pub(crate) fn from_code(code: u32) -> Self {
        Self {
            code: Some(code as i32),
            signal: None,
        }
    }

    /// The command was terminated by `signal`.
    pub(crate) fn from_signal(signal: impl Into<String>) -> Self {
        Self {
            code: None,
            signal: Some(signal.into()),
        }
    }

    /// The exit code, when the command exited normally.
    pub fn code(&self) -> Option<i32> {
        self.code
    }

    /// The signal name (e.g. `"KILL"`), when the command was terminated by one.
    pub fn signal(&self) -> Option<&str> {
        self.signal.as_deref()
    }

    /// Whether the command succeeded.
    ///
    /// A missing exit status counts as success, matching `ssh(1)`: some
    /// servers close the channel without sending one. A signal is always a
    /// failure.
    pub fn success(&self) -> bool {
        self.signal.is_none() && self.code.unwrap_or(0) == 0
    }
}

impl std::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.signal, self.code) {
            (Some(signal), _) => write!(f, "signal {signal}"),
            (None, Some(code)) => write!(f, "exit status {code}"),
            (None, None) => f.write_str("exit status unknown"),
        }
    }
}

/// Captured output of a remote command, like [`std::process::Output`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use = "Output contains the exit status — check it"]
pub struct Output {
    /// Exit status of the command.
    pub status: ExitStatus,
    /// Raw bytes written to stdout.
    pub stdout: Vec<u8>,
    /// Raw bytes written to stderr.
    pub stderr: Vec<u8>,
}

impl Output {
    /// Whether the command exited successfully.
    pub fn success(&self) -> bool {
        self.status.success()
    }

    /// The exit code, when the command exited normally.
    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    /// Decode stdout as UTF-8, replacing invalid sequences.
    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Decode stderr as UTF-8, replacing invalid sequences.
    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_status_is_success() {
        assert!(ExitStatus::default().success());
        assert_eq!(ExitStatus::default().code(), None);
    }

    #[test]
    fn codes_map_to_success() {
        assert!(ExitStatus::from_code(0).success());
        assert!(!ExitStatus::from_code(1).success());
        assert_eq!(ExitStatus::from_code(7).code(), Some(7));
    }

    #[test]
    fn signals_are_failures() {
        let status = ExitStatus::from_signal("KILL");
        assert!(!status.success());
        assert_eq!(status.signal(), Some("KILL"));
        assert_eq!(status.to_string(), "signal KILL");
    }

    #[test]
    fn output_decodes_lossily() {
        let out = Output {
            stdout: b"ok\xff".to_vec(),
            ..Output::default()
        };
        assert!(out.stdout_lossy().starts_with("ok"));
        assert!(out.success());
    }
}
