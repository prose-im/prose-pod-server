// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! See [prosodyctl shell – Prosody IM](https://prosody.im/doc/console).

use std::process::Stdio;

use anyhow::{Context as _, anyhow};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::Duration;

#[derive(Debug, Default)]
pub struct ProsodyShell {
    handle: Option<ProsodyShellHandle>,
}

#[derive(Debug)]
struct ProsodyShellHandle {
    process: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

#[derive(Debug, Default)]
pub struct ProsodyResponse {
    pub lines: Vec<String>,
    pub summary: Option<String>,
}

impl ProsodyShell {
    pub fn new() -> Self {
        Self::default()
    }

    fn start_shell_<'a>(&'a mut self) -> anyhow::Result<&'a mut ProsodyShellHandle> {
        let mut child = Command::new("prosodyctl")
            .arg("shell")
            .arg("--quiet")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed spawning prosodyctl")?;

        let stdin = (child.stdin.take()).ok_or(anyhow!("Failed to get prosodyctl stdin"))?;
        let stdout = (child.stdout.take()).ok_or(anyhow!("Failed to get prosodyctl stdout"))?;
        let reader = BufReader::new(stdout).lines();

        let handle = self.handle.insert(ProsodyShellHandle {
            process: child,
            stdin,
            stdout: reader,
        });

        Ok(handle)
    }

    /// Get shell handle, starting the shell if needed.
    fn get_handle_or_start<'a>(&'a mut self) -> anyhow::Result<&'a mut ProsodyShellHandle> {
        match self.handle {
            Some(ref mut handle) => Ok(handle),
            None => self.start_shell_(),
        }
    }

    /// 200ms is enough: Prosody is fast and running on the same machine.
    const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

    /// Execute a command.
    pub async fn exec(&mut self, command: &str) -> anyhow::Result<ProsodyResponse> {
        self.exec_with_timeout(command, Self::DEFAULT_TIMEOUT).await
    }

    const MAX_COMMAND_LENGTH: usize = 1024;

    /// Execute a command with a custom timeout.
    pub async fn exec_with_timeout(
        &mut self,
        command: &str,
        timeout: Duration,
    ) -> anyhow::Result<ProsodyResponse> {
        // Check input.
        assert!(!command.is_empty());
        assert!(command.len() < Self::MAX_COMMAND_LENGTH);

        // Get or start the shell.
        let handle = self.get_handle_or_start()?;

        // Log command (without args since they can contain sensitive data).
        {
            // TODO: Create a wrapper for that implements `tracing::Value` and
            //   logs the full command when in debug mode (and only then).
            let command = command_name(command);
            tracing::trace!("Running command `{command}`…");
            tracing::Span::current().record("command", command);
        }

        // Send command.
        handle.stdin.write_all(command.as_bytes()).await?;
        if !command.ends_with('\n') {
            handle.stdin.write_u8(b'\n').await?;
        }
        handle.stdin.flush().await?;

        // Some constants to improve readability.
        const FIRST_LINE_PREFIX: &'static str = "prosody> ";
        const ERROR_LINE_PREFIX: &'static str = "! ";
        const SUMMARY_LINE_PREFIX: &'static str = "| OK: ";
        const RESPONSE_LINE_PREFIX: &'static str = "| ";

        // Read response.
        let mut response = ProsodyResponse::default();
        while let Some(full_line) = tokio::time::timeout(timeout, handle.stdout.next_line())
            .await
            .context("Timeout")?
            .context("I/O error")?
        {
            // Remove first line prefix.
            let line = if full_line.starts_with(FIRST_LINE_PREFIX) {
                &full_line[FIRST_LINE_PREFIX.len()..]
            } else {
                &full_line
            };

            // Parse the response.
            if line.starts_with(ERROR_LINE_PREFIX) {
                let error_msg = &line[ERROR_LINE_PREFIX.len()..];
                return Err(anyhow!(error_msg.to_owned()));
            } else if line.starts_with(SUMMARY_LINE_PREFIX) {
                let summary = &line[SUMMARY_LINE_PREFIX.len()..];
                response.summary = Some(summary.to_owned());
                break;
            } else if line.starts_with(RESPONSE_LINE_PREFIX) {
                let line = &line[RESPONSE_LINE_PREFIX.len()..];
                response.lines.push(line.to_owned());
            } else if line.contains("warn\t") {
                // NOTE: Prosody can show a warning on stdout when reading
                //   its configuration file. It might look like:
                //
                //   ```log
                //   startup             warn\tConfiguration warning: /etc/prosody/prosody.cfg.lua:42: Duplicate option 'foo'
                //   ```
                //
                //   When we encounter it, we can just forward the log
                //   and skip the line.
                tracing::warn!("[prosody]: {line}");
            } else {
                if cfg!(debug_assertions) {
                    // Crash in debug mode to avoid missing such cases.
                    return Err(anyhow!("Got unexpected result line: {line:?}."));
                } else {
                    tracing::error!("[prosody]: {line}");
                }
            }
        }

        Ok(response)
    }
}

// MARK: - Convenience methods for common operations

impl ProsodyShell {
    pub async fn list_users(&mut self, domain: &str) -> anyhow::Result<Vec<String>> {
        let response = self
            .exec(&format!("user:list(\"{}\")", domain))
            .await
            .context("Error listing users")?;
        Ok(response.lines)
    }

    pub async fn create_user(&mut self, jid: &str, password: &str) -> anyhow::Result<String> {
        let response = self
            .exec(&format!("user:create(\"{}\", \"{}\")", jid, password))
            .await
            .context("Error creating user")?;
        Ok(response.summary.unwrap_or_else(|| format!("Created {jid}")))
    }

    pub async fn delete_user(&mut self, jid: &str) -> anyhow::Result<String> {
        let response = self
            .exec(&format!("user:delete(\"{}\")", jid))
            .await
            .context("Error deleting user")?;
        Ok(response.summary.unwrap_or_else(|| format!("Deleted {jid}")))
    }
}

// MARK: - Plumbing

impl Drop for ProsodyShell {
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

// MARK: - Helpers

/// Command without args since they can contain sensitive data.
///
/// E.g.: `user:create("test@admin.prose.local", "password")` -> `user:create`.
fn command_name<'a>(command: &'a str) -> &'a str {
    assert!(command.contains("("));

    // Ensure the command is not using the
    // [shortcut syntax](https://prosody.im/doc/console#shortcut).
    let paren_idx = command.find("(").unwrap();
    if let Some(space_idx) = command.find(" ") {
        assert!(space_idx > paren_idx);
    }

    &command[..paren_idx]
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_command_name() {
        use super::command_name;

        let command = r#"user:create("test@admin.prose.local", "password")"#;
        assert_eq!(command_name(command), "user:create");
    }
}
