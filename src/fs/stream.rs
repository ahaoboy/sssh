//! Streaming transfers for payloads that should not be held in memory.
//!
//! [`Fs::read`] and [`Fs::write`] buffer the whole file in a `Vec<u8>`, which
//! is convenient for dotfiles but wasteful for images, archives or firmware.
//! These helpers copy bytes through the SSH channel instead, like
//! [`std::io::copy`].

use std::io;
use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite};

use super::{Fs, quote};

impl Fs {
    /// Stream a remote file into `out`, returning the number of bytes written
    /// (like [`std::io::copy`]).
    ///
    /// ```no_run
    /// # async fn run(fs: sssh::Fs, local: tokio::fs::File) -> std::io::Result<()> {
    /// let mut local = local;
    /// let copied = fs.download("/var/log/syslog", &mut local).await?;
    /// println!("{copied} bytes");
    /// # Ok(()) }
    /// ```
    pub async fn download<W>(&self, path: impl AsRef<Path>, out: W) -> io::Result<u64>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let target = self.resolve(path);
        let script = format!("cat {}", quote(&target));
        let streamed = self
            .client()
            .stream(&script, tokio::io::empty(), out, tokio::io::sink(), false)
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
        if !streamed.output.success() {
            return Err(self.failure("download", &target, &streamed.output));
        }
        Ok(streamed.received)
    }

    /// Stream `src` into `path`, truncating any existing file, and return the
    /// number of bytes written.
    ///
    /// `src` is moved into a background writer, hence `'static`;
    /// `tokio::fs::File` and in-memory cursors satisfy this.
    pub async fn upload<R>(&self, path: impl AsRef<Path>, src: R) -> io::Result<u64>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        self.upload_with_mode(path, src, None).await
    }

    /// Like [`Self::upload`], applying `mode` (e.g. `0o755`) once the data has
    /// been written.
    pub async fn upload_with_mode<R>(
        &self,
        path: impl AsRef<Path>,
        src: R,
        mode: Option<u32>,
    ) -> io::Result<u64>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let target = self.resolve(path);
        let quoted = quote(&target);

        let mut script = format!("cat > {quoted}");
        if let Some(mode) = mode.filter(|mode| *mode > 0) {
            script.push_str(&format!(" && chmod {:o} {quoted}", mode & 0o7777));
        }
        // Report the resulting size so a short write is detected below.
        script.push_str(&format!(" && wc -c < {quoted} | tr -d ' \\n'"));

        let streamed = self
            .client()
            .stream(&script, src, tokio::io::sink(), tokio::io::sink(), true)
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
        let out = &streamed.output;
        if !out.success() {
            return Err(self.failure("upload", &target, out));
        }

        let written: u64 = out.stdout_lossy().trim().parse().unwrap_or(0);
        if !streamed.finished || written != streamed.sent {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!(
                    "wrote {written} of {} bytes to '{target}' on {} (transfer interrupted or disk full)",
                    streamed.sent,
                    self.client().label()
                ),
            ));
        }
        Ok(written)
    }
}
