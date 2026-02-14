// prosodyctl-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! See [prosodyctl shell – Prosody IM](https://prosody.im/doc/console).

use std::process::Stdio;

use anyhow::{Context as _, anyhow};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{Duration, Instant};

use crate::Password;

pub mod errors {
    pub use super::UserCreateError;
}

#[derive(Debug)]
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
    /// NOTE: This constructor is lazy. The shell will start
    ///   when you make the first request.
    #[must_use]
    pub fn new() -> Self {
        Self { handle: None }
    }

    /// Get shell handle, starting the shell if needed.
    #[must_use]
    async fn get_handle_or_start<'a>(&'a mut self) -> anyhow::Result<&'a mut ProsodyShellHandle> {
        match self.handle {
            Some(ref mut handle_ref) => Ok(handle_ref),
            None => {
                let handle = ProsodyShellHandle::new().await?;
                let handle_ref = self.handle.insert(handle);
                Ok(handle_ref)
            }
        }
    }

    /// 200ms is enough: Prosody is fast and running on the same machine.
    const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

    /// Some actions are O(n^2) and take longer. This timeout can be used then.
    const LONG_TIMEOUT: Duration = Duration::from_secs(10);

    /// If a command takes more than this duration to execute,
    /// log execution time.
    const EXEC_LOG_THRESHOLD: Duration = Duration::from_millis(1000);

    /// Execute a command.
    #[must_use]
    pub async fn exec(&mut self, command: &str) -> anyhow::Result<ProsodyResponse> {
        self.exec_with_timeout(command, Self::DEFAULT_TIMEOUT).await
    }

    const MAX_COMMAND_LENGTH: usize = 1024;

    /// Execute a command with a custom timeout.
    #[must_use]
    pub async fn exec_with_timeout(
        &mut self,
        command: &str,
        timeout: Duration,
    ) -> anyhow::Result<ProsodyResponse> {
        let start = Instant::now();

        // Get or start the shell.
        let handle = self.get_handle_or_start().await?;

        // Execute command.
        let res = handle.exec_with_timeout(command, timeout).await;

        // Log execution time if too long.
        let elapsed = start.elapsed();
        if elapsed > Self::EXEC_LOG_THRESHOLD {
            let command = command_name(command);
            tracing::warn!("Command `{command}` took {elapsed:.0?} to execute.");
        }

        res
    }
}

impl ProsodyShellHandle {
    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, err)]
    async fn new() -> anyhow::Result<Self> {
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

        let mut handle = ProsodyShellHandle {
            process: child,
            stdin,
            stdout: reader,
        };

        // Import utilites to simplify commands later.
        let imports = vec![
            r#"> it = require"prosody.util.iterators""#,
            r#"> dump = require"prosody.util.serialization".new({ preset = "oneline" })"#,
            r#"> mm = require"core.modulemanager""#,
            r#"> um = require"core.usermanager""#,
        ];
        let timeout = ProsodyShell::DEFAULT_TIMEOUT;
        for import in imports {
            (handle.exec_with_timeout(import, timeout))
                .await
                .context("Error when running `require`")?;
        }

        Ok(handle)
    }

    /// Execute a command with a custom timeout.
    #[tracing::instrument(level = "trace", skip_all, err)]
    async fn exec_with_timeout(
        &mut self,
        command: &str,
        timeout: Duration,
    ) -> anyhow::Result<ProsodyResponse> {
        // Check input.
        assert!(!command.is_empty());
        assert!(command.len() < ProsodyShell::MAX_COMMAND_LENGTH);

        // Log command (without args since they can contain sensitive data).
        {
            // TODO: Create a wrapper for that implements `tracing::Value` and
            //   logs the full command when in debug mode (and only then).
            let command = command_name(command);
            tracing::trace!("Running command `{command}`…");
            tracing::Span::current().record("command", command);
        }

        // Send command.
        tracing::trace!("[>] {command}");
        self.stdin.write_all(command.as_bytes()).await?;
        if !command.ends_with('\n') {
            self.stdin.write_u8(b'\n').await?;
        }
        self.stdin.flush().await?;

        // Some constants to improve readability.
        const FIRST_LINE_PREFIX: &'static str = "prosody> ";
        // NOTE: E.g. “** Unable to connect to server - is it running? Is mod_admin_shell enabled?”.
        const EXCEPTION_LINE_PREFIX: &'static str = "** ";
        // NOTE: E.g. “! console:1: attempt to index a nil value (global 'mm')”
        //   or “! /lib/prosody/core/usermanager.lua:125: attempt to index a nil value (field '?')”.
        const ERROR_LINE_PREFIX: &'static str = "! ";
        // NOTE: When a function returns `nil, "message"`.
        //   E.g. “! Error: Auth failed. Invalid username”.
        const ERROR_RESULT_PREFIX: &'static str = "! Error: ";
        const SUMMARY_LINE_PREFIX: &'static str = "| OK: ";
        const RESULT_LINE_PREFIX: &'static str = "| Result: ";
        const LOG_LINE_PREFIX: &'static str = "| ";

        // Read response.
        let mut lines: Vec<String> = Vec::new();
        let mut result: Option<Result<String, String>> = None;
        while let Some(full_line) = tokio::time::timeout(timeout, self.stdout.next_line())
            .await
            .context("Timeout")?
            .context("I/O error")?
        {
            tracing::trace!("[<] {full_line}");

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
            } else if line.starts_with(ERROR_RESULT_PREFIX) {
                let error_msg = &line[ERROR_RESULT_PREFIX.len()..];
                result = Some(Err(error_msg.to_owned()));
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
                // NOTE: Prosody can show a warning on stdout when starting up.
                //   It might look like:
                //
                //   ```log
                //   startup             warn\tConfiguration warning: /etc/prosody/prosody.cfg.lua:42: Duplicate option 'foo'
                //   ```
                //
                //   When we encounter it, we can just forward the log
                //   and skip the line.
                tracing::warn!("[prosody]: {line}");
            } else if line.contains("error\t") {
                // NOTE: Prosody can show an error on stdout when starting up.
                //   It might look like:
                //
                //   ```log
                //   certmanager         error\tError indexing certificate directory /etc/prosody/certs: cannot open /etc/prosody/certs: No such file or directory
                //   ```
                //
                //   When we encounter it, abort.
                return Err(anyhow!("Prosody error: {line:?}"));
            } else {
                if cfg!(debug_assertions) {
                    // Raise error in debug mode to avoid missing such cases.
                    return Err(anyhow!("Got unexpected result line: {line:?}"));
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

// Miscellaneous
impl ProsodyShell {
    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, fields(host), err)]
    pub async fn host_exists(&mut self, host: &str) -> anyhow::Result<bool> {
        let command = format!(r#"> not not prosody.hosts["{host}"]"#);

        let response = (self.exec(&command))
            .await
            .context("Error testing if host exists")?;

        response
            .result_bool()
            .context(command_name(&command).to_owned())
            .context("Error testing if host exists")
    }

    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, err)]
    pub async fn prosody_paths_config(&mut self) -> anyhow::Result<String> {
        let command = format!(r#"> prosody.paths.config"#);

        let response = (self.exec(&command))
            .await
            .context("Error getting Prosody config path")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, err)]
    pub async fn prosody_paths_data(&mut self) -> anyhow::Result<String> {
        let command = format!(r#"> prosody.paths.data"#);

        let response = (self.exec(&command))
            .await
            .context("Error getting Prosody data path")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Waits for Prosody to be ready after it starts.
    ///
    /// While starting up, Prosody loads modules and runs some initialization
    /// logic. Sometimes, if a command which should be instantaneous is ran too
    /// soon it ends up timing out.
    /// This ensures Prosody is ready to receive shell commands.
    ///
    /// Note that this method is empirical and might not work reliably.
    /// It seems to work, but I (@RemiBardon) don’t really know why.
    #[tracing::instrument(level = "trace", skip_all, err)]
    pub async fn wait_for_readiness(&mut self) -> anyhow::Result<()> {
        let command = format!(r#"> not not prosody"#);

        self.exec_with_timeout(&command, Self::LONG_TIMEOUT)
            .await
            .context("Error waiting for Prosody readiness")
            .map(drop)
    }
}

// usermanager
impl ProsodyShell {
    /// Lists users on the specified host, optionally filtering with a pattern.
    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, fields(domain, pattern), err)]
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
    #[tracing::instrument(level = "trace", skip_all, fields(username, host), err)]
    pub async fn user_exists(&mut self, username: &str, host: &str) -> anyhow::Result<bool> {
        debug_assert!(!username.contains("@"));

        if !self.host_exists(host).await? {
            return Err(anyhow!("Host '{host}' does not exit."));
        }

        let command = format!(r#"> um.user_exists("{username}", "{host}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error testing if user account exists")?;

        response
            .result_bool_expecting(["Auth failed. Invalid username"].into_iter())
            .context(command_name(&command).to_owned())
            .context("Error testing if user account exists")
    }

    /// Creates the specified user account, with an optional primary role
    /// assigned to it right away.
    ///
    /// WARN: Raises an error if the user already exists (does not update the
    ///   password like `usermanager.create_user` does II(@RemiBardon)RC).
    #[tracing::instrument(level = "trace", skip_all, fields(jid, role), err)]
    pub async fn user_create(
        &mut self,
        jid: &str,
        password: &Password,
        role: Option<&str>,
    ) -> Result<String, UserCreateError> {
        #[cfg(feature = "secrecy")]
        let password = secrecy::ExposeSecret::expose_secret(password);

        let command = match role {
            None => format!(r#"user:create("{jid}", "{password}")"#),
            Some(role) => format!(r#"user:create("{jid}", "{password}", "{role}")"#),
        };

        let response = (self.exec(&command))
            .await
            .context("Error creating user account")?;

        Ok(response.result?)
    }

    /// Sets the password for the specified user account.
    #[tracing::instrument(level = "trace", skip_all, fields(jid), err)]
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
    #[tracing::instrument(level = "trace", skip_all, fields(jid, host), err)]
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
    #[tracing::instrument(level = "trace", skip_all, fields(jid, host, new_role), err)]
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
    #[tracing::instrument(level = "trace", skip_all, fields(jid), err)]
    pub async fn user_disable(&mut self, jid: &str) -> anyhow::Result<String> {
        let command = format!(r#"user:disable("{jid}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error disabling user account")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Enables the specified user account, restoring login access.
    #[tracing::instrument(level = "trace", skip_all, fields(jid), err)]
    pub async fn user_enable(&mut self, jid: &str) -> anyhow::Result<String> {
        let command = format!(r#"user:enable("{jid}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error enabling user account")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Permanently removes the specified user account.
    #[tracing::instrument(level = "trace", skip_all, fields(jid), err)]
    pub async fn user_delete(&mut self, jid: &str) -> anyhow::Result<String> {
        let command = format!(r#"user:delete("{jid}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error deleting user account")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    #[tracing::instrument(level = "trace", skip(self), err)]
    pub async fn user_get_jids_with_role(
        &mut self,
        host: &str,
        role_name: &str,
    ) -> anyhow::Result<Vec<String>> {
        if !self.host_exists(host).await? {
            return Err(anyhow!("Host '{host}' does not exit."));
        }

        let command = format!(r#"> dump(um.get_jids_with_role("{role_name}", "{host}"))"#);

        let response = (self.exec_with_timeout(&command, Self::LONG_TIMEOUT))
            .await
            .context("Error listing enabled modules")?;

        response.result_string_array()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UserCreateError {
    #[error("User already exists.")]
    Conflict,
    #[error("{0:#}")]
    Internal(#[from] anyhow::Error),
}

impl From<String> for UserCreateError {
    fn from(error: String) -> Self {
        match error.as_str() {
            "User exists" => Self::Conflict,
            _ => Self::Internal(anyhow::Error::msg(error)),
        }
    }
}

// modulemanager
impl ProsodyShell {
    #[tracing::instrument(level = "trace", skip_all, fields(module, host), err)]
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

    #[tracing::instrument(level = "trace", skip_all, fields(module, host), err)]
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

    #[tracing::instrument(level = "trace", skip_all, fields(module, host), err)]
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
    #[tracing::instrument(level = "trace", skip_all, fields(host, module), err)]
    pub async fn module_is_loaded(&mut self, host: &str, module: &str) -> anyhow::Result<bool> {
        if host != "*" && !self.host_exists(host).await? {
            return Err(anyhow!("Host '{host}' does not exit."));
        }

        let command = format!(r#"> mm.is_loaded("{host}", "{module}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error testing if module is loaded")?;

        response
            .result_bool()
            .context(command_name(&command).to_owned())
            .context("Error testing if module is loaded")
    }

    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, fields(host), err)]
    pub async fn module_list(&mut self, host: &str) -> anyhow::Result<Vec<String>> {
        if host != "*" && !self.host_exists(host).await? {
            return Err(anyhow!("Host '{host}' does not exit."));
        }

        // NOTE: We can’t just use `modulemanager.get_modules_for_host` as that
        //   only returns *enabled* modules, and not the ones loaded via an
        //   admin interface.
        let command = format!(r#"> dump(it.to_array(it.keys(mm.get_modules("{host}"))))"#);

        let response = (self.exec(&command))
            .await
            .context("Error listing modules")?;

        response.result_string_array()
    }

    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, fields(host), err)]
    pub async fn module_list_enabled(&mut self, host: &str) -> anyhow::Result<Vec<String>> {
        if !self.host_exists(host).await? {
            return Err(anyhow!("Host '{host}' does not exit."));
        }

        let command = format!(r#"> dump(it.to_array(mm.get_modules_for_host("{host}")))"#);

        let response = (self.exec(&command))
            .await
            .context("Error listing enabled modules")?;

        response.result_string_array()
    }

    #[must_use]
    #[tracing::instrument(level = "trace", skip_all, fields(host), err)]
    pub async fn module_load_modules_for_host(&mut self, host: &str) -> anyhow::Result<()> {
        if !self.host_exists(host).await? {
            return Err(anyhow!("Host '{host}' does not exit."));
        }

        let command = format!(r#"> mm.load_modules_for_host("{host}")"#);

        let reponse = self
            .exec(&command)
            .await
            .context("Error loading modules for host")?;

        reponse.result_unit()
    }

    /// Just an internal helper.
    #[inline]
    #[tracing::instrument(level = "trace", skip_all, fields(host, module), err)]
    async fn require_module(&mut self, host: &str, module: &str) -> anyhow::Result<()> {
        if self.module_is_loaded(host, module).await? {
            Ok(())
        } else {
            if tracing::enabled!(tracing::Level::DEBUG) {
                match self.module_list(host).await {
                    Ok(loaded) => {
                        tracing::debug!("Loaded modules for '{host}': {loaded:?}");
                    }
                    Err(err) => {
                        tracing::error!("Could not list loaded modules for '{host}': {err}")
                    }
                };
                match self.module_list_enabled(host).await {
                    Ok(enabled) => {
                        tracing::debug!("Enabled modules for '{host}': {enabled:?}");
                    }
                    Err(err) => {
                        tracing::error!("Could not list enabled modules for '{host}': {err}")
                    }
                };
            }
            Err(anyhow!("Module '{module}' not loaded for '{host}'."))
        }
    }
}

#[cfg(feature = "mod_groups")]
impl ProsodyShell {
    #[tracing::instrument(level = "trace", skip(self), err)]
    pub async fn groups_create(
        &mut self,
        host: &str,
        group_name: &str,
        create_default_muc: Option<bool>,
        group_id: Option<&str>,
    ) -> anyhow::Result<String> {
        self.require_module("*", "groups_shell").await?;

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

    #[tracing::instrument(level = "trace", skip(self), err)]
    pub async fn groups_add_member(
        &mut self,
        host: &str,
        group_id: &str,
        username: &str,
        delay_update: Option<bool>,
    ) -> anyhow::Result<String> {
        self.require_module("*", "groups_shell").await?;

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

    #[tracing::instrument(level = "trace", skip(self), err)]
    pub async fn groups_sync(&mut self, host: &str, group_id: &str) -> anyhow::Result<String> {
        self.require_module("*", "groups_shell").await?;

        let command = format!(r#"groups:sync_group("{host}", "{group_id}")"#);

        let response = (self.exec_with_timeout(&command, Self::LONG_TIMEOUT))
            .await
            .context("Error synchronizing group")?;

        Ok(response.result.map_err(anyhow::Error::msg)?)
    }

    /// Tests if a group exists on a given host.
    ///
    /// NOTE: Does not require `mod_groups_shell` to be loaded.
    #[must_use]
    #[tracing::instrument(level = "trace", skip(self), err)]
    pub async fn groups_exists(&mut self, host: &str, group_id: &str) -> anyhow::Result<bool> {
        self.require_module(host, "groups_internal").await?;

        let command =
            format!(r#"> mm.get_module("{host}", "groups_internal").exists("{group_id}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error testing if group exists")?;

        response
            .result_bool()
            .context(command_name(&command).to_owned())
            .context("Error testing if group exists")
    }
}

#[cfg(feature = "mod_invites")]
impl ProsodyShell {
    // NOTE: Example output for `prosodyctl shell invite list example.org`:
    //
    //   ```txt
    //   Token                    | Expires              | Description
    //   tCydHSBO9PmORhiNW3Xb2b3u | 2025-10-06T16:05:34  | Register on example.org with username test
    //   SPmxTuui_2u48UcI3BY-Qz9c | 2025-10-06T16:04:01  | Register on example.org
    //   OK: 2 pending invites
    //   ```
    #[tracing::instrument(level = "trace", skip(self), err)]
    pub async fn invite_list(&mut self, host: &str) -> anyhow::Result<Vec<InviteRow>> {
        self.require_module(host, "invites").await?;

        let command = format!(r#"invite:list("{host}")"#);

        let response = (self.exec(&command))
            .await
            .context("Error listing invites")?;

        let res = response.lines.into_iter().skip(1).map(|row| {
            let mut cells = row
                .splitn(3, " | ")
                .map(|cell| cell.trim_ascii().to_owned());
            InviteRow {
                token: cells.next().unwrap(),
                expires_at: cells.next().unwrap(),
                description: cells.next().unwrap(),
            }
        });

        Ok(res.collect())
    }
}

pub struct InviteRow {
    pub token: String,
    pub expires_at: String,
    pub description: String,
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
    pub fn result_unit(self) -> anyhow::Result<()> {
        match self.result {
            Ok(_) => Ok(()),
            Err(err) => Err(anyhow::Error::msg(err)),
        }
    }

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

    /// In Lua, it is common for functions acting as boolean checks
    /// to return `nil` instead of `false`. However, this results in
    /// the shell raising an error instead of returning `false`/`nil`
    /// (e.g. `usermanager.user_exists`). If we know in advance what
    /// errors would be sent instead, we can do the mapping here.
    #[must_use]
    #[inline]
    pub fn result_bool_expecting<'a>(
        &self,
        mut expected_errors: impl Iterator<Item = &'a str>,
    ) -> anyhow::Result<bool> {
        match self.result.as_ref() {
            Err(err) if expected_errors.any(|e| e.eq(err)) => Ok(false),
            _ => self.result_bool(),
        }
    }

    #[must_use]
    #[inline]
    pub fn result_string_array(&self) -> anyhow::Result<Vec<String>> {
        let result = (self.result)
            .as_ref()
            .map_err(|err| anyhow::Error::msg(err.clone()))?;

        Ok(lua_string_to_string_array(result))
    }
}

fn lua_string_to_array_iter(lua: &str) -> impl Iterator<Item = &str> {
    assert!(
        lua.starts_with("{"),
        "Lua array should start with `{{`. Got `{lua}`."
    );
    assert!(
        lua.ends_with("}"),
        "Lua array should end with `}}`. Got `{lua}`. Make sure to call `dump` to force formatting on one line."
    );
    assert!(
        !lua.contains('\n'),
        "Lua array should not contain any `\\n` (use `dump`). Got `{lua}`."
    );

    lua[1..(lua.len() - 1)]
        .trim()
        .split("; ")
        // Skip first empty strings.
        // NOTE: While this might create unexpected results, it’s good enough
        //   for now as it will likely only ever appear when the input string
        //   is empty (what we are trying to handle).
        .skip_while(|s| s.is_empty())
}

fn lua_string_to_string_array(lua: &str) -> Vec<String> {
    (lua_string_to_array_iter(lua))
        .map(|s| s.trim_matches('"'))
        .map(ToOwned::to_owned)
        .collect()
}

/// Command without args since they can contain sensitive data.
///
/// E.g.: `user:create("test@admin.prose.local", "password")` -> `user:create`.
#[must_use]
#[inline]
fn command_name<'a>(command: &'a str) -> &'a str {
    let marker_idx = if let Some(paren_idx) = command.find("(") {
        // NOTE: For cases like `> mm.is_loaded("example.org", "group_shell")`.
        paren_idx
    } else if let Some(bracket_idx) = command.find("[") {
        // NOTE: For cases like `> not not prosody.hosts["example.org"]`.
        bracket_idx
    } else if command.contains("require") {
        // NOTE: For cases like `> mm = require"core.modulemanager"`.
        command.len()
    } else if command.contains("\"") {
        panic!(
            "Command `{command}` potentially contains sensitive information. \
            Add a case to support it."
        )
    } else {
        command.len()
    };

    // Check if using the
    // [advanced syntax](https://prosody.im/doc/console#advanced_usage).
    let is_advanced_syntax = command.starts_with(">");

    // Ensure the command is not using the
    // [shortcut syntax](https://prosody.im/doc/console#shortcut).
    if !is_advanced_syntax {
        // NOTE: `command.find("(").expect` isn’t enough.
        //   For example, a password could contain a `(`.
        if let Some(space_idx) = command.find(" ") {
            assert!(space_idx > marker_idx);
        }
    }

    &command[..marker_idx]
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_command_name() {
        use super::command_name;

        // Base syntax.
        let command = r#"user:create("test@example.org", "password")"#;
        assert_eq!(command_name(command), "user:create");

        // Advanced syntax.
        let command = r#"> require"core.modulemanager".is_loaded("example.org", "group_shell")"#;
        assert_eq!(
            command_name(command),
            r#"> require"core.modulemanager".is_loaded"#,
        );

        // Advanced cases
        let advanced = vec![
            (
                r#"> require"core.modulemanager".is_loaded("example.org", "group_shell")"#,
                r#"> require"core.modulemanager".is_loaded"#,
            ),
            (
                r#"> not not prosody.hosts["example.org"]"#,
                r#"> not not prosody.hosts"#,
            ),
            (
                r#"> mm = require"core.modulemanager""#,
                r#"> mm = require"core.modulemanager""#,
            ),
        ];
        for (command, name) in advanced.into_iter() {
            assert_eq!(command_name(command), name, "command: {command}");
        }
    }

    #[test]
    fn test_lua_parse_array() {
        use super::lua_string_to_string_array;

        // NOTE: The format is enforced by `prosody.util.serialization`’s
        //   preset `"oneline"` (imported as `dump`).
        let cases = vec![
            (r#"{}"#, vec![]),
            (r#"{ "offline" }"#, vec!["offline"]),
            (
                r#"{ "offline"; "presence"; "c2s" }"#,
                vec![
                    "offline", "presence", "c2s",
                ],
            ),
        ];
        for (lua, rust) in cases {
            assert_eq!(lua_string_to_string_array(lua), rust, "lua: {lua}");
        }
    }
}
