// prosodyctl-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! See [prosodyctl shell – Prosody IM](https://prosody.im/doc/console).

use std::process::Stdio;

use anyhow::{Context as _, anyhow};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::Duration;

use crate::Password;

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

#[derive(Debug)]
pub struct ProsodyResponse {
    pub lines: Vec<String>,
    pub result: Result<String, String>,
}

impl ProsodyShell {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    #[inline]
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
    #[must_use]
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
        // NOTE: E.g. “** Unable to connect to server - is it running? Is mod_admin_shell enabled?”
        const EXCEPTION_LINE_PREFIX: &'static str = "** ";
        const ERROR_LINE_PREFIX: &'static str = "! ";
        const SUMMARY_LINE_PREFIX: &'static str = "| OK: ";
        const RESULT_LINE_PREFIX: &'static str = "| Result: ";
        const LOG_LINE_PREFIX: &'static str = "| ";

        // Read response.
        let mut lines: Vec<String> = Vec::new();
        let mut result: Option<Result<String, String>> = None;
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
            if line.starts_with(EXCEPTION_LINE_PREFIX) {
                let exception_msg = &line[EXCEPTION_LINE_PREFIX.len()..];
                result = Some(Err(exception_msg.to_owned()));
                break;
            } else if line.starts_with(ERROR_LINE_PREFIX) {
                let error_msg = &line[ERROR_LINE_PREFIX.len()..];
                result = Some(Err(error_msg.to_owned()));
                break;
            } else if line.starts_with(SUMMARY_LINE_PREFIX) {
                let summary = &line[SUMMARY_LINE_PREFIX.len()..];
                result = Some(Ok(summary.to_owned()));
                break;
            } else if line.starts_with(RESULT_LINE_PREFIX) {
                let res = &line[RESULT_LINE_PREFIX.len()..];
                result = Some(Ok(res.to_owned()));
                break;
            } else if line.starts_with(LOG_LINE_PREFIX) {
                let line = &line[LOG_LINE_PREFIX.len()..];
                lines.push(line.to_owned());
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
                    // Raise error in debug mode to avoid missing such cases.
                    return Err(anyhow!("Got unexpected result line: {line:?}."));
                } else {
                    tracing::error!("[prosody]: {line}");
                }
            }
        }

        match result {
            Some(result) => Ok(ProsodyResponse { lines, result }),
            None => Err(anyhow!("Got no result line.")),
        }
    }
}

// MARK: - Convenience methods for common operations

// usermanager
impl ProsodyShell {
    /// Lists users on the specified host, optionally filtering with a pattern.
    #[must_use]
    pub async fn user_list(
        &mut self,
        domain: &str,
        pattern: Option<&str>,
    ) -> anyhow::Result<Vec<String>> {
        let command = match pattern {
            None => format!(r#"user:list("{domain}")"#),
            Some(pattern) => format!(r#"user:list("{domain}", "{pattern}")"#),
        };

        let response = (self.exec(&command))
            .await
            .context("Error listing user accounts")?;

        Ok(response.lines)
    }

    /// Tests if the specified user account exists.
    #[must_use]
    pub async fn user_exists(&mut self, username: &str, host: &str) -> anyhow::Result<bool> {
        let command = format!(r#"> require"core.usermanager".user_exists("{username}", "{host}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error testing if user account exists")?;

        let result = response
            .result_bool()
            .context(command_name(&command).to_owned())
            .context("Error testing if user account exists")?;

        Ok(result)
    }

    /// Creates the specified user account, with an optional primary role
    /// assigned to it right away.
    ///
    /// WARN: Raises an error if the user already exists (does not update the
    ///   password like `usermanager.create_user` does II(@RemiBardon)RC).
    pub async fn user_create(
        &mut self,
        jid: &str,
        password: &Password,
        role: Option<&str>,
    ) -> anyhow::Result<String> {
        #[cfg(feature = "secrecy")]
        let password = secrecy::ExposeSecret::expose_secret(password);

        let command = match role {
            None => format!(r#"user:create("{jid}", "{password}")"#),
            Some(role) => format!(r#"user:create("{jid}", "{password}", "{role}")"#),
        };

        let response = (self.exec(&command))
            .await
            .context("Error creating user account")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Sets the password for the specified user account.
    pub async fn user_password(
        &mut self,
        jid: &str,
        password: &Password,
    ) -> anyhow::Result<String> {
        #[cfg(feature = "secrecy")]
        let password = secrecy::ExposeSecret::expose_secret(password);

        let command = format!(r#"user:password("{jid}", "{password}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error setting user account password")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Shows the primary role for a user.
    #[must_use]
    pub async fn user_role(&mut self, jid: &str, host: Option<&str>) -> anyhow::Result<String> {
        let command = match host {
            None => format!(r#"user:role("{jid}")"#),
            Some(host) => format!(r#"user:role("{jid}", "{host}")"#),
        };

        let response = (self.exec(&command))
            .await
            .context("Error getting user primary role")?;

        let mut role = response.result.map_err(anyhow::Error::msg)?;

        // NOTE: If the user has secondary roles, “ (primary)” is appended to
        //   the user role in the result line. This removes it.
        if let Some(slice) = role.strip_suffix(" (primary)") {
            role = slice.to_owned();
        }

        Ok(role)
    }

    /// Sets the primary role of a user.
    pub async fn user_set_role(
        &mut self,
        jid: &str,
        host: Option<&str>,
        new_role: &str,
    ) -> anyhow::Result<String> {
        let command = match host {
            None => format!(r#"user:set_role("{jid}", "{new_role}")"#),
            Some(host) => format!(r#"user:set_role("{jid}", "{host}", "{new_role}")"#),
        };

        let response = (self.exec(&command))
            .await
            .context("Error setting user primary role")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Disables the specified user account, preventing login.
    pub async fn user_disable(&mut self, jid: &str) -> anyhow::Result<String> {
        let command = format!(r#"user:disable("{jid}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error disabling user account")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Enables the specified user account, restoring login access.
    pub async fn user_enable(&mut self, jid: &str) -> anyhow::Result<String> {
        let command = format!(r#"user:enable("{jid}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error enabling user account")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Permanently removes the specified user account.
    pub async fn user_delete(&mut self, jid: &str) -> anyhow::Result<String> {
        let command = format!(r#"user:delete("{jid}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error deleting user account")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }
}

// modulemanager
impl ProsodyShell {
    pub async fn module_load(
        &mut self,
        module: &str,
        host: Option<&str>,
    ) -> anyhow::Result<String> {
        let command = match host {
            None => format!(r#"module:load("{module}")"#),
            Some(host) => format!(r#"module:load("{module}", "{host}")"#),
        };

        let response = (self.exec(&command))
            .await
            .context("Error loading module")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    pub async fn module_unload(
        &mut self,
        module: &str,
        host: Option<&str>,
    ) -> anyhow::Result<String> {
        let command = match host {
            None => format!(r#"module:unload("{module}")"#),
            Some(host) => format!(r#"module:unload("{module}", "{host}")"#),
        };

        let response = (self.exec(&command))
            .await
            .context("Error unloading module")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    pub async fn module_reload(
        &mut self,
        module: &str,
        host: Option<&str>,
    ) -> anyhow::Result<String> {
        let command = match host {
            None => format!(r#"module:reload("{module}")"#),
            Some(host) => format!(r#"module:reload("{module}", "{host}")"#),
        };

        let response = (self.exec(&command))
            .await
            .context("Error reloading module")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    #[must_use]
    pub async fn module_is_loaded(&mut self, host: &str, module: &str) -> anyhow::Result<bool> {
        let command = format!(r#"> require"core.modulemanager".is_loaded("{host}", "{module}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error testing if module is loaded")?;

        response.result_bool()
    }

    /// Just an internal helper.
    #[inline]
    async fn require_module(&mut self, host: &str, module: &str) -> anyhow::Result<()> {
        if self.module_is_loaded(host, module).await? {
            Ok(())
        } else {
            Err(anyhow!("Module '{module}' not loaded for '{host}'."))
        }
    }
}

#[cfg(feature = "mod_groups")]
impl ProsodyShell {
    pub async fn groups_create(
        &mut self,
        host: &str,
        group_name: &str,
        create_default_muc: Option<bool>,
        group_id: Option<&str>,
    ) -> anyhow::Result<String> {
        self.require_module(host, "groups_shell").await?;

        let command = match (create_default_muc, group_id) {
            (None, None) => format!(r#"groups:create("{host}", "{group_name}")"#),
            (Some(create_default_muc), None) => {
                format!(r#"groups:create("{host}", "{group_name}", {create_default_muc})"#)
            }
            (Some(create_default_muc), Some(group_id)) => format!(
                r#"groups:create("{host}", "{group_name}", {create_default_muc}, "{group_id}")"#
            ),
            (None, Some(group_id)) => {
                format!(r#"groups:create("{host}", "{group_name}", nil, "{group_id}")"#)
            }
        };

        let response = (self.exec(&command))
            .await
            .context("Error creating group")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    pub async fn groups_add_member(
        &mut self,
        host: &str,
        group_id: &str,
        username: &str,
        delay_update: Option<bool>,
    ) -> anyhow::Result<String> {
        self.require_module(host, "groups_shell").await?;

        let command = match delay_update {
            None => format!(r#"groups:add_member("{host}", "{group_id}", "{username}")"#),
            Some(delay_update) => format!(
                r#"groups:add_member("{host}", "{group_id}", "{username}", "{delay_update}")"#
            ),
        };

        let response = (self.exec(&command))
            .await
            .context("Error adding group member")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    pub async fn groups_sync(&mut self, host: &str, group_id: &str) -> anyhow::Result<String> {
        self.require_module(host, "groups_shell").await?;

        let command = format!(r#"groups:sync_group("{host}", "{group_id}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error synchronizing group")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Tests if a group exists on a given host.
    ///
    /// NOTE: Does not require `mod_groups_shell` to be loaded.
    #[must_use]
    pub async fn groups_exists(&mut self, host: &str, group_id: &str) -> anyhow::Result<bool> {
        let command = format!(
            r#"> require"core.modulemanager".get_module("{host}", "groups_internal").exists("{group_id}")"#
        );

        let response = (self.exec(&command))
            .await
            .context("Error testing if group exists")?;

        Ok(response.result_bool()?)
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

impl ProsodyResponse {
    #[must_use]
    #[inline]
    pub fn result_bool(&self) -> anyhow::Result<bool> {
        let result = (self.result)
            .as_ref()
            .map_err(|err| anyhow::Error::msg(err.clone()))?;

        match result.as_str() {
            "true" => Ok(true),
            "false" | "nil" => Ok(false),
            res => {
                if cfg!(debug_assertions) {
                    // Raise error in debug mode to avoid missing such cases.
                    let error_msg = format!("Got unexpected boolean result: '{res}'.");
                    tracing::error!("{error_msg}");
                    Err(anyhow::Error::msg(error_msg))
                } else {
                    Ok(false)
                }
            }
        }
    }
}

/// Command without args since they can contain sensitive data.
///
/// E.g.: `user:create("test@admin.prose.local", "password")` -> `user:create`.
#[must_use]
#[inline]
fn command_name<'a>(command: &'a str) -> &'a str {
    assert!(command.contains("("));
    let paren_idx = command
        .find("(")
        .expect("Commands should use the default or advanced syntaxes.");

    // Check if using the
    // [advanced syntax](https://prosody.im/doc/console#advanced_usage).
    let is_advanced_syntax = command.starts_with(">");

    // Ensure the command is not using the
    // [shortcut syntax](https://prosody.im/doc/console#shortcut).
    if !is_advanced_syntax {
        // NOTE: `command.find("(").expect` isn’t enough.
        //   For example, a password could contain a `(`.
        if let Some(space_idx) = command.find(" ") {
            assert!(space_idx > paren_idx);
        }
    }

    &command[..paren_idx]
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_command_name() {
        use super::command_name;

        let command = r#"user:create("test@example.org", "password")"#;
        assert_eq!(command_name(command), "user:create");

        let command = r#"> require"core.modulemanager".is_loaded("example.org", "group_shell")"#;
        assert_eq!(
            command_name(command),
            r#"> require"core.modulemanager".is_loaded"#,
        );
    }
}
