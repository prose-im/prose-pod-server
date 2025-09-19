// prosodyctl-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! See [prosodyctl – Prosody IM](https://prosody.im/doc/prosodyctl).

use std::process::Stdio;

use anyhow::anyhow;
use tokio::{process::Command, task::JoinHandle};

use crate::prosody_shell::ProsodyShell;

/// See [prosodyctl – Prosody IM](https://prosody.im/doc/prosodyctl).
pub struct Prosodyctl {
    shell: ProsodyShell,
}

impl Prosodyctl {
    pub fn new() -> Self {
        Self {
            shell: ProsodyShell::new(),
        }
    }

    /// Start Prosody in the foreground (blocking).
    pub fn start(&self) -> JoinHandle<anyhow::Result<()>> {
        let mut cmd = {
            let mut cmd = Command::new("prosody");

            // Don’t daemonize.
            cmd.arg("--foreground");

            cmd
        };

        tokio::spawn(async move {
            cmd.stdout(Stdio::null());

            let status = cmd.status().await?;

            if status.success() {
                Ok(())
            } else {
                Err(anyhow!(
                    "Prosody exited with code: {code:?}",
                    code = status.code(),
                ))
            }
        })
    }

    /// Check if Prosody is already running.
    pub async fn is_running(&self) -> bool {
        // Try to connect to the telnet console as a health check.
        use std::net::TcpStream;
        use std::time::Duration;

        TcpStream::connect_timeout(&"127.0.0.1:5582".parse().unwrap(), Duration::from_secs(1))
            .is_ok()
    }

    /// Stop Prosody gracefully.
    pub async fn stop(&self) -> anyhow::Result<()> {
        let output = Command::new("prosodyctl").arg("stop").output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Failed to stop Prosody: {stderr}"));
        }

        tracing::info!("Prosody stopped successfully");
        Ok(())
    }

    /// Restart Prosody.
    pub async fn restart(&self) -> anyhow::Result<()> {
        let output = Command::new("prosodyctl").arg("restart").output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Failed to restart Prosody: {stderr}"));
        }

        tracing::info!("Prosody restarted successfully");
        Ok(())
    }
}

impl std::ops::Deref for Prosodyctl {
    type Target = ProsodyShell;

    fn deref(&self) -> &Self::Target {
        &self.shell
    }
}

impl std::ops::DerefMut for Prosodyctl {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.shell
    }
}
