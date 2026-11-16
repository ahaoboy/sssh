//! Remote system information (`uname(1)`).
//!
//! Useful to pick a release artifact for the remote host without shipping a
//! platform-detection script:
//!
//! ```no_run
//! # async fn run(client: sssh::Client) -> Result<(), sssh::Error> {
//! let uname = client.uname().await?;
//! println!("{} {} ({})", uname.sysname, uname.release, uname.machine);
//! # Ok(()) }
//! ```

use crate::client::Client;
use crate::error::Error;

/// The fields reported by `uname(1)` on the remote host.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Uname {
    /// Operating system name (`uname -s`), e.g. `Linux` or `Darwin`.
    pub sysname: String,
    /// Network node hostname (`uname -n`).
    pub nodename: String,
    /// Kernel release (`uname -r`).
    pub release: String,
    /// Kernel version (`uname -v`).
    pub version: String,
    /// Hardware machine (`uname -m`), e.g. `x86_64` or `aarch64`.
    pub machine: String,
    /// Operating system (`uname -o`), when the host implements it.
    ///
    /// GNU/BSD `uname` report values such as `GNU/Linux`; BusyBox builds
    /// without `-o` leave this as `None`.
    pub os: Option<String>,
}

impl Uname {
    /// Parse the `KEY=value` output of the probe script.
    fn parse(stdout: &str) -> Self {
        let mut uname = Self::default();
        for line in stdout.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().to_string();
            match key.trim() {
                "SYSNAME" => uname.sysname = value,
                "NODENAME" => uname.nodename = value,
                "RELEASE" => uname.release = value,
                "VERSION" => uname.version = value,
                "MACHINE" => uname.machine = value,
                "OS" => uname.os = (!value.is_empty()).then_some(value),
                _ => {}
            }
        }
        uname
    }
}

impl Client {
    /// Run `uname(1)` on the remote host.
    ///
    /// One round trip; `uname` is invoked through `sh`, so the result does
    /// not depend on the login shell (fish, zsh, …).
    pub async fn uname(&self) -> Result<Uname, Error> {
        let output = self.exec_sh(UNAME_SCRIPT).await?;
        let uname = Uname::parse(&output.stdout_lossy());
        if uname.sysname.is_empty() {
            return Err(Error::Other(format!(
                "`uname -s` returned nothing on {} ({})",
                self.label(),
                output.status
            )));
        }
        Ok(uname)
    }
}

const UNAME_SCRIPT: &str = r#"
printf 'SYSNAME=%s\n' "$(uname -s 2>/dev/null)"
printf 'NODENAME=%s\n' "$(uname -n 2>/dev/null)"
printf 'RELEASE=%s\n' "$(uname -r 2>/dev/null)"
printf 'VERSION=%s\n' "$(uname -v 2>/dev/null)"
printf 'MACHINE=%s\n' "$(uname -m 2>/dev/null)"
printf 'OS=%s\n' "$(uname -o 2>/dev/null)"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uname_output() {
        let uname = Uname::parse(
            "SYSNAME=Linux\nNODENAME=router\nRELEASE=6.8.0\nVERSION=#1 SMP\n\
             MACHINE=aarch64\nOS=GNU/Linux\n",
        );
        assert_eq!(uname.sysname, "Linux");
        assert_eq!(uname.nodename, "router");
        assert_eq!(uname.release, "6.8.0");
        assert_eq!(uname.version, "#1 SMP");
        assert_eq!(uname.machine, "aarch64");
        assert_eq!(uname.os.as_deref(), Some("GNU/Linux"));
    }

    #[test]
    fn missing_os_is_none() {
        let uname = Uname::parse("SYSNAME=Linux\nMACHINE=x86_64\nOS=\n");
        assert_eq!(uname.os, None);
    }

    #[test]
    fn ignores_unknown_lines() {
        let uname = Uname::parse("EXTRA=1\nSYSNAME=Darwin\n");
        assert_eq!(uname.sysname, "Darwin");
    }
}
