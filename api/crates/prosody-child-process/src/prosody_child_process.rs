// prosody-child-process-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    process::Stdio,
};

use anyhow::{Context as _, anyhow};
use nix::{
    sys::signal::{Signal::SIGHUP, kill},
    unistd::Pid,
};
use tokio::process::{Child, Command};

use crate::util::{debug_panic, debug_panic_or_log_warning};

#[derive(Debug)]
pub struct ProsodyChildProcess {
    handle: Option<ProsodyHandle>,
    envs: HashMap<OsString, OsString>,
    /// A unique ID that’s used in debug logs to differenciate
    /// which instance is “speaking”.
    id: UniqueId,
}

#[derive(Debug)]
struct ProsodyHandle {
    process: Child,
    // stdout: Lines<BufReader<ChildStdout>>,
}

impl ProsodyChildProcess {
    /// NOTE: This constructor is lazy. Prosody will start when you call
    ///   [`ProsodyChildProcess::start`].
    #[inline]
    pub fn new() -> Self {
        Self {
            handle: None,
            envs: HashMap::new(),
            id: UniqueId::new(),
        }
    }

    /// Stores a new environment variable to attach to Prosody next time you
    /// call [`start`](Self::start).
    ///
    /// Beware that changes are not applied to running Prosody instances.
    /// You need to [`restart`](Self::restart) to apply changes in that case.
    ///
    /// Also note that environment variables are unique, with last insert
    /// taking precedence.
    #[inline]
    pub fn set_env<K: AsRef<OsStr>, V: AsRef<OsStr>>(&mut self, key: K, value: V) {
        self.envs
            .insert(key.as_ref().to_owned(), value.as_ref().to_owned());
    }

    /// Equivalent of [`set_env`](Self::set_env) but
    /// returning the new value to support chaining.
    #[inline]
    pub fn env<K: AsRef<OsStr>, V: AsRef<OsStr>>(mut self, key: K, value: V) -> Self {
        self.set_env(key, value);
        self
    }

    /// Start Prosody in the background (non blocking).
    pub async fn start(&mut self) -> Result<(), anyhow::Error> {
        tracing::debug!(instance = %self.id, "Starting Prosody…");

        // Start Prosody (as a child process).
        let mut handle = ProsodyHandle::new(self.envs.clone().into_iter()).await?;

        // Check if Prosody started successfully.
        //
        // Prosody fails fast, therefore if it did not exit after 250ms,
        // it likely means everything went well.
        //
        // TODO: Once we pipe Prosody logs to redirect it to `tracing`,
        //   read the logs and exit as soon as the first `stdout` line
        //   says “welcome to Prosody” (to avoid constant wait).
        let exit_status = tokio::time::timeout(
            tokio::time::Duration::from_millis(250),
            handle.process.wait(),
        )
        .await;
        match exit_status {
            Ok(Ok(status)) => {
                return Err(anyhow!(
                    "Prosody did not start successfully: Exited early ({status})."
                ));
            }
            Ok(Err(err)) => {
                let err = format!("Failed waiting for Prosody exit: {err:#}");
                debug_panic(&err);
                return Err(anyhow!(err));
            }
            Err(_) => {
                // Prosody is still running -> it started successfully.
            }
        }

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
        tracing::debug!(instance = %self.id, "Stopping Prosody…");

        let Some(mut handle) = self.handle.take() else {
            debug_panic_or_log_warning("Not stopping Prosody: No handle (likely already stopped).");
            return Ok(());
        };

        handle.process.kill().await?;

        // Wait for Prosody to terminate.
        // NOTE: Prosody can still save data after it’s been killed,
        //   during its graceful shutdown process. This ensures Prosody
        //   is inert after this function ends.
        handle.process.wait().await?;

        tracing::info!(instance = %self.id, "Prosody stopped successfully.");
        Ok(())
    }

    /// Reload Prosody.
    pub async fn reload(&mut self) -> Result<(), anyhow::Error> {
        tracing::debug!(instance = %self.id, "Reloading Prosody…");

        let Some(handle) = self.handle.as_ref() else {
            debug_panic_or_log_warning("Prosody not started: No handle (likely stopped).");
            return self.start().await;
        };

        let Some(pid) = handle.process.id() else {
            debug_panic_or_log_warning("Prosody not started: No PID (likely stopped).");
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
    async fn new(envs: impl Iterator<Item = (OsString, OsString)>) -> Result<Self, anyhow::Error> {
        let child = Command::new("prosody")
            .arg("--no-daemonize")
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .envs(envs)
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

// MARK: - Debug helpers

#[derive(Clone, Copy)]
struct UniqueId(u16);

impl UniqueId {
    #[inline]
    pub fn new() -> Self {
        // `16^4` -> formatting as hexadecimal will yield 4 characters.
        // `16^4 == 2^16` -> we can always fit the number in a `u16`.
        // `(16^4)/3600/24 ≈ 0,76` -> ids will loop every `3÷4` day
        // (likely no collision ever if >1s between two calls).
        Self((crate::util::unix_timestamp() % 16u64.pow(4)) as u16)
    }
}

impl std::fmt::Display for UniqueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04x}", self.0)
    }
}

impl std::fmt::Debug for UniqueId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04x}", self.0)
    }
}

// MARK: - Plumbing

impl Drop for ProsodyChildProcess {
    fn drop(&mut self) {
        if let Some(mut handle) = self.handle.take() {
            let instance = self.id;

            // Move ownership of child into a background task.
            tokio::spawn(async move {
                tracing::debug!(%instance, "[Drop] Killing Prosody…");

                // Try killing gracefully.
                handle.process.kill().await.unwrap_or_else(|err| {
                    tracing::error!(%instance, "Could not kill long-running `prosodyctl shell`: {err}")
                });

                // Wait to reap the process (avoids zombies).
                let _ = handle.process.wait().await;
            });
        }
    }
}
