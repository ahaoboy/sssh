//! SSH client handler with configurable host-key verification.

use russh::client::Handler;
use russh::keys::ssh_key;
use std::sync::Arc;
use tracing::{debug, warn};

/// Policy for verifying the server's host key.
///
/// Implement this trait to plug in a custom verification strategy
/// (e.g. TOFU, known-hosts file, interactive prompt). The default
/// implementation — [`AcceptAll`] — mimics `StrictHostKeyChecking=no`.
pub trait HostKeyVerifier: Send + Sync {
    /// Return `Ok(())` to accept the key, or `Err(msg)` to reject.
    fn verify(&self, host: &str, key: &ssh_key::PublicKey) -> Result<(), String>;
}

/// Accept **every** host key without checking (insecure).
///
/// This is the default for backward compatibility but should be
/// replaced with a stricter verifier in production.
pub struct AcceptAll;

impl HostKeyVerifier for AcceptAll {
    fn verify(&self, _host: &str, _key: &ssh_key::PublicKey) -> Result<(), String> {
        debug!("Accepting server host key (AcceptAll policy)");
        Ok(())
    }
}

/// SSH client handler.
pub(crate) struct ClientHandler {
    pub host: String,
    pub verifier: Arc<dyn HostKeyVerifier>,
}

impl Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        match self.verifier.verify(&self.host, server_public_key) {
            Ok(()) => {
                debug!("Server host key accepted for {}", self.host);
                Ok(true)
            }
            Err(msg) => {
                warn!("Server host key REJECTED for {}: {msg}", self.host);
                Ok(false)
            }
        }
    }
}
