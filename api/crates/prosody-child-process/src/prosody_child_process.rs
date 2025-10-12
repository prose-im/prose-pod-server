// prosody-child-process-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::process::Stdio;

use anyhow::Context as _;
use nix::{
    sys::signal::{Signal::SIGHUP, kill},
    unistd::Pid,
};
use tokio::process::{Child, Command};

use crate::util::debug_panic_or_log_warning;

#[derive(Debug)]
pub struct ProsodyChildProcess {
    handle: Option<ProsodyHandle>,
}

#[derive(Debug)]
struct ProsodyHandle {
    process: Child,
    // stdout: Lines<BufReader<ChildStdout>>,
}

impl ProsodyChildProcess {
    pub fn new() -> Self {
        Self { handle: None }
    }

    /// Start Prosody in the background (non blocking).
    pub async fn start(&mut self) -> Result<(), anyhow::Error> {
        let handle = ProsodyHandle::new().await?;

        self.handle = Some(handle);

        Ok(())
    }

    /// Check if Prosody is already running.
    pub async fn is_running(&self) -> bool {
        // Try to connect to the telnet console as a health check.
        use std::net::{Ipv4Addr, TcpStream};
        use std::time::Duration;

        TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, 5582).into(), Duration::from_secs(1))
            .is_ok()
    }

    /// Stop Prosody gracefully.
    pub async fn stop(&mut self) -> Result<(), anyhow::Error> {
        let Some(mut handle) = self.handle.take() else {
            debug_panic_or_log_warning("Not stopping Prosody: No handle (likely already stopped).");
            return Ok(());
        };

        handle.process.kill().await?;

        tracing::info!("Prosody stopped successfully.");
        Ok(())
    }

    /// Reload Prosody.
    pub async fn reload(&mut self) -> Result<(), anyhow::Error> {
        let Some(handle) = self.handle.take() else {
            debug_panic_or_log_warning("Prosody not started: No handle (likely already stopped).");
            return self.start().await;
        };

        let Some(pid) = handle.process.id() else {
            debug_panic_or_log_warning("Prosody not started: No PID (likely already stopped).");
            return self.start().await;
        };

        kill(Pid::from_raw(pid as i32), SIGHUP)?;

        Ok(())
    }

    /// Restart Prosody.
    pub async fn restart(&mut self) -> Result<(), anyhow::Error> {
        self.stop().await?;
        self.start().await?;
        Ok(())
    }
}

impl ProsodyHandle {
    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, err)]
    async fn new() -> Result<Self, anyhow::Error> {
        let child = Command::new("prosody")
            .arg("--no-daemonize")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed spawning prosody")?;

        // let stdout = (child.stdout.take()).ok_or(anyhow!("Failed to get prosody stdout"))?;
        // let reader = BufReader::new(stdout).lines();

        let handle = ProsodyHandle {
            process: child,
            // stdin,
            // stdout: reader,
        };

        Ok(handle)
    }
}

// MARK: - Plumbing

impl Drop for ProsodyChildProcess {
    fn drop(&mut self) {
        if let Some(mut handle) = self.handle.take() {
            // Move ownership of child into a background task
            tokio::spawn(async move {
                // Try killing gracefully.
                handle.process.kill().await.unwrap_or_else(|err| {
                    tracing::error!("Could not kill long-running `prosodyctl shell`: {err}")
                });
                // Wait to reap the process (avoids zombies).
                let _ = handle.process.wait().await;
            });
        }
    }
}
