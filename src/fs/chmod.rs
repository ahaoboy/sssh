//! Changing permission bits.
//!
//! Backed by `chmod`, so modes behave like [`std::fs::set_permissions`] on a
//! Unix remote (including setuid/setgid/sticky bits).

use std::io;
use std::path::Path;

use super::types::Permissions;
use super::{Fs, quote};

impl Fs {
    /// Set permissions (POSIX mode bits) on `path`, like
    /// [`std::fs::set_permissions`].
    pub async fn set_permissions(
        &self,
        path: impl AsRef<Path>,
        permissions: Permissions,
    ) -> io::Result<()> {
        self.set_mode(path, permissions.mode()).await
    }

    /// Set POSIX mode bits (e.g. `0o755`) on `path`.
    ///
    /// Only the permission bits (`0o7777`) are kept; higher bits in `mode`
    /// are ignored.
    pub async fn set_mode(&self, path: impl AsRef<Path>, mode: u32) -> io::Result<()> {
        let target = self.resolve(path);
        let script = format!("chmod {:o} {}", mode & 0o7777, quote(&target));
        self.run_unit("chmod", &target, &script).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_script_drops_bits_above_7777() {
        let script = format!("chmod {:o} {}", 0o100755 & 0o7777, quote("/tmp/tool"));
        assert_eq!(script, "chmod 755 '/tmp/tool'");
    }
}
