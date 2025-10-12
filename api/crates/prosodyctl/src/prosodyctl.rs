// prosodyctl-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! See [prosodyctl – Prosody IM](https://prosody.im/doc/prosodyctl).

use anyhow::anyhow;
use tokio::process::Command;

use crate::prosody_shell::ProsodyShell;

/// See [prosodyctl – Prosody IM](https://prosody.im/doc/prosodyctl).
#[derive(Debug)]
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
    pub async fn start(&self) -> anyhow::Result<()> {
        let output = Command::new("prosodyctl").arg("start").output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Failed to stop Prosody: {stderr}"));
        }

        tracing::info!("Prosody started successfully");
        Ok(())
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
