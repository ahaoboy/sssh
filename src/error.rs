//! Error type for the crate.

/// All errors produced by `sssh` operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SSH connection failed: {0}")]
    Connection(#[from] russh::Error),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Host key verification failed: {0}")]
    HostKeyVerification(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Connection timed out after {0}s")]
    Timeout(u64),

    #[error("{0}")]
    Other(String),
}
