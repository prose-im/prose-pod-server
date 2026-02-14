// prose-pod-server
//
// Copyright: 2025–2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;
use std::fs::{self, remove_dir_all};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use arc_swap::ArcSwap;
use prosody_child_process::ProsodyChildProcess;
use prosody_http::ProsodyHttpConfig;
use prosody_http::oauth2::{self, OAuth2ClientConfig, ProsodyOAuth2};
use prosody_rest::ProsodyRest;
use prosodyctl::Prosodyctl;
use secrecy::SecretSlice;
use tokio::sync::RwLock;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::models::{BareJid, JidDomain, JidNode, Password};
use crate::secrets_service::SecretsService;
use crate::secrets_store::SecretsStore;
use crate::state::prelude::*;
use crate::util::unix_timestamp;
use crate::{AppConfig, errors};

pub(crate) const SERVER_DATA_DIR: &'static str = "/var/lib/prose-pod-server";
const PROSODY_CONFIG_FILE_PATH: &'static str = "/etc/prosody/prosody.cfg.lua";
const PROSODY_CERTS_DIR: &'static str = "/etc/prosody/certs";

// MARK: - State transitions

impl AppState<f::Running, b::Starting> {
    /// Try bootstrapping the backend, but do not transition if an error occurs.
    /// See [docs/bootstrapping.md](../docs/bootstrapping.md) for information
    /// about bootstrapping.
    ///
    /// NOTE: This method does **not** log errors.
    async fn bootstrap(app_state: &Self) -> Result<b::Running, anyhow::Error> {
        use crate::util::sync::AutoCancelToken;

        let app_config = Arc::deref(&app_state.frontend.config);
        let ref server_domain = app_config.server.domain;

        create_required_dirs()?;

        let server_salt = generate_server_salt_if_needed()?;

        backup_prosody_conf_if_needed()?;

        apply_bootstrap_config(server_domain)?;

        // NOTE: While it’s here that we could delete the `localhost` data
        //   generated during a factory reset, it’s better to not do it to
        //   avoid data loss. If someone starts a Prose Pod on existing
        //   Prosody data while they had important things stored in `localhost`,
        //   they’d be horrified if we deleted their data without consent.
        //   We’ll keep destructive actions during factory resets only.
        //   It will make the Prose Pod Server more predictable, safer to use
        //   and faster to audit (only one place where `fs::remove_dir_all`
        //   happens).

        // Launch Prosody.
        let mut prosody = ProsodyChildProcess::new();
        start_prosody(&app_state.frontend.config, &mut prosody).await?;

        let mut prosodyctl = Prosodyctl::new();

        prosodyctl.wait_for_readiness().await?;

        let cancellation_token = CancellationToken::new();

        let prosody_rest = ProsodyRest::standard(app_config.server.http_url());

        let prosody_http_config = Arc::new(ProsodyHttpConfig {
            url: "http://prose-pod-server:5280".to_owned(),
        });
        let oauth2_client = Arc::new(ProsodyOAuth2::new(prosody_http_config));

        // TODO: Allow avoiding registration by passing
        //   client credentials via configuration.
        let oauth2_client_credentials = register_oauth2_client(&oauth2_client).await?;

        let secrets = SecretsService {
            store: SecretsStore::new(app_config),
            oauth2: oauth2_client.clone(),
            oauth2_client_credentials: ArcSwap::from_pointee(oauth2_client_credentials),
        };
        // Run cache purge tasks in the background.
        tokio::spawn(secrets.run_purge_tasks(cancellation_token.child_token()));

        let service_accounts = create_service_accounts(
            app_config,
            &mut prosodyctl,
            &prosody_rest,
            &oauth2_client,
            &secrets,
        )
        .await?;

        let groups = Groups::new(app_config.as_ref());
        create_groups(&mut prosodyctl, &groups, server_domain).await?;

        {
            let service_accounts_usernames = service_accounts
                .iter()
                .flat_map(|jid| jid.node().map(JidNode::from));
            let group_ids = groups.keys().into_iter();
            add_service_accounts_to_groups(
                &mut prosodyctl,
                service_accounts_usernames,
                group_ids,
                server_domain,
            )
            .await?;
        }

        {
            let group_ids = groups.keys().into_iter();
            synchronize_rosters(&mut prosodyctl, group_ids, server_domain).await?;
        }

        let backend = b::Running {
            state: Arc::new(b::Operational {
                prosody: Arc::new(RwLock::new(prosody)),
                prosodyctl: Arc::new(RwLock::new(prosodyctl)),
                prosody_rest,
                oauth2_client,
                secrets_service: secrets,
                http_client: reqwest::Client::new(),
                server_salt,
                cancellation_token: AutoCancelToken(cancellation_token),
            }),
        };

        run_migrations(app_config, &backend).await?;

        Ok(backend)
    }

    /// Try bootstrapping the backend, but do not transition if an error occurs.
    /// See [docs/bootstrapping.md](../docs/bootstrapping.md) for information
    /// about bootstrapping.
    ///
    /// NOTE: This method does **not** log errors.
    pub(crate) async fn try_bootstrapping(
        self,
    ) -> Result<AppState<f::Running, b::Running>, (Self, anyhow::Error)> {
        tracing::info!("Bootstrapping…");
        let start = Instant::now();

        match Self::bootstrap(&self).await {
            Ok(backend) => {
                let new_state = self.with_backend(backend).with_auto_transition();

                tracing::info!("Bootstrapping took {:.0?}.", start.elapsed());
                Ok(new_state)
            }
            Err(err) => {
                let error = err.context("Bootstrapping failed");

                tracing::info!("Bootstrapping failed in {:.0?}.", start.elapsed());
                Err((self, error))
            }
        }
    }

    /// Bootstrap the backend or transition to a fail state if an error occurs.
    /// See [docs/bootstrapping.md](../docs/bootstrapping.md) for information
    /// about bootstrapping.
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) async fn do_bootstrapping(
        self,
    ) -> Result<AppState<f::Running, b::Running>, FailState<f::Running, b::StartFailed>> {
        match self.try_bootstrapping().await {
            Ok(new_state) => Ok(new_state),

            Err((app_state, error)) => {
                // Log debug info.
                tracing::error!("{error:?}");

                Err(app_state.transition_failed(errors::internal_server_error(
                    &error,
                    "START_FAILED",
                    "Something went wrong while starting your Prose Server. \
                    Contact an administrator to fix this.",
                )))
            }
        }
    }
}

// MARK: - Steps

fn create_required_dirs() -> Result<(), anyhow::Error> {
    fs::create_dir_all(SERVER_DATA_DIR).context(format!(
        "Could not create Prose Pod Server data dir at <{path}>",
        path = SERVER_DATA_DIR,
    ))?;
    fs::create_dir_all(PROSODY_CERTS_DIR).context(format!(
        "Could not create Prosody certs dir at <{path}>",
        path = PROSODY_CERTS_DIR,
    ))?;

    Ok(())
}

#[must_use]
fn generate_server_salt_if_needed() -> Result<SecretSlice<u8>, anyhow::Error> {
    use crate::util::random_bytes;
    use std::fs::File;
    use std::io::{Read as _, Write as _};

    let salt_path = server_salt_path();

    const SALT_LENGTH: usize = 256;

    if salt_path.exists() {
        let mut file = File::open(salt_path).context("Error opening Server salt file")?;

        // COMPAT: Read to `Vec` and not `[u8; 256]` as length might change
        //   in a future version.
        let mut salt = Vec::with_capacity(SALT_LENGTH);
        file.read_to_end(&mut salt)
            .context("Error reading Server salt")?;

        Ok(SecretSlice::from(salt))
    } else {
        let salt = random_bytes::<SALT_LENGTH>();

        let mut file = File::create_new(salt_path).context("Error opening Server salt file")?;
        file.write_all(&salt).context("Error writing Server salt")?;

        Ok(SecretSlice::from(salt.to_vec()))
    }
}

fn backup_prosody_conf_if_needed() -> Result<(), anyhow::Error> {
    use std::fs::File;
    use std::io::{self, Read as _};

    let prosody_config_file_path = Path::new(PROSODY_CONFIG_FILE_PATH);

    // Back up the Prosody configuration if it was not generated by Prose.
    // This is just to avoid a bad surprise to anyone deploying Prose on an
    // existing Prosody instance.
    match File::options().read(true).open(prosody_config_file_path) {
        Ok(mut prosody_config_file) => {
            let prose_header = "-- Prose Pod Server";
            let mut buffer = vec![0u8; prose_header.len()];

            // Read the first few bytes to check the header.
            let bytes_read = prosody_config_file
                .read(&mut buffer)
                .context("Error reading Prosody config file")?;
            buffer.truncate(bytes_read);

            if buffer != prose_header.as_bytes() {
                let mut new_path = prosody_config_file_path.to_path_buf();
                let unix_timestamp = unix_timestamp();
                new_path.set_file_name(format!("prosody.prose-backup-{unix_timestamp}.cfg.lua"));

                tracing::info!(
                    "The Prosody configuration file at <{old_path}> was not generated by Prose. \
                    To prevent data loss, it will be backed up as <{new_path}>.",
                    old_path = prosody_config_file_path.display(),
                    new_path = new_path.display(),
                )
            }

            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // Prosody config file does not exist already, nothing to back up.
            Ok(())
        }
        Err(err) => Err(anyhow::Error::new(err).context(format!(
            "Error opening <{path}>",
            path = prosody_config_file_path.display(),
        ))),
    }
}

fn apply_bootstrap_config(server_domain: &JidDomain) -> Result<(), anyhow::Error> {
    use std::fs::File;
    use std::io::Write as _;

    let mut prosody_config_file = File::options()
        .write(true)
        .truncate(true)
        .create(true)
        .open(PROSODY_CONFIG_FILE_PATH)
        .context("Error opening Prosody config file")?;

    let bootstrap_config_template = include_str!("prosody-bootstrap.cfg.lua");

    let bootstrap_config = bootstrap_config_template.replace("{{server_domain}}", server_domain);

    prosody_config_file
        .write_all(bootstrap_config.as_bytes())
        .context("Error writing Prosody config file")?;

    Ok(())
}

async fn start_prosody(
    app_config: &AppConfig,
    prosody: &mut ProsodyChildProcess,
) -> Result<(), anyhow::Error> {
    use secrecy::ExposeSecret as _;

    prosody.set_env(
        "OAUTH2_REGISTRATION_KEY",
        app_config.auth.oauth2_registration_key.expose_secret(),
    );
    prosody.start().await.context("Failed starting Prosody")?;

    Ok(())
}

async fn register_oauth2_client(
    oauth2: &ProsodyOAuth2,
) -> Result<oauth2::ClientCredentials, anyhow::Error> {
    let oauth2_client_config = OAuth2ClientConfig {
        client_name: "Prose Pod Server API".to_owned(),
        client_uri: "https://prose-pod-server:8080".to_owned(),
        redirect_uris: vec!["https://prose-pod-server:8080/redirect".to_owned()],
        grant_types: vec![
            "authorization_code".to_owned(),
            "refresh_token".to_owned(),
            "password".to_owned(),
        ],
        ..Default::default()
    };

    let client_metadata = oauth2.register(&oauth2_client_config).await?;

    // Make sure the client credentials never expire. Otherwise we’d need
    // a token refresh process which doesn’t exist yet.
    debug_assert_eq!(client_metadata.client_secret_expires_at, 0);

    Ok(client_metadata.into_credentials())
}

/// Creates the “prose-workspace” user for now, maybe more later.
async fn create_service_accounts(
    app_config: &AppConfig,
    prosodyctl: &mut Prosodyctl,
    prosody_rest: &ProsodyRest,
    oauth2: &ProsodyOAuth2,
    secrets: &SecretsService,
) -> Result<Vec<BareJid>, anyhow::Error> {
    use prosody_rest::prose_xmpp::stanza::{VCard4, vcard4};

    // NOTE: [Prosody’s built-in roles](https://prosody.im/doc/roles#built-in-roles)
    //   don’t have a concept of non-user account. Until we have our own roles,
    //   we will create service accounts as if it were normal users.
    // TODO: Use special role for service accounts.
    let role = "prosody:registered";

    // Read service accounts credentials from app configuration.
    let accounts: Vec<(BareJid, String, Option<Password>)> = vec![(
        app_config.workspace_jid(),
        app_config.server.domain.to_string(),
        app_config.service_accounts.prose_workspace.password.clone(),
    )];

    // Lock credentials with exclusive write access
    // to prevent race conditions.
    let mut passwords_guard = secrets.passwords_rw_guard().await;
    let mut tokens_guard = secrets.tokens_rw_guard().await;

    for (jid, name, password_opt) in accounts.iter() {
        let password = password_opt.clone().unwrap_or_else(Password::random);

        let Some(username) = jid.node() else {
            tracing::warn!("Service account `{jid}` has no node part. Can’t create it.");
            continue;
        };

        // Create the account if needed, or update password.
        if prosodyctl.user_exists(username, jid.domain()).await? {
            tracing::debug!("Setting user `{jid}` password…");
            let summary = prosodyctl.user_password(jid.as_str(), &password).await?;
            tracing::info!("user_password: {summary}");

            tracing::debug!("Setting user `{jid}` role…");
            let summary = prosodyctl.user_set_role(jid.as_str(), None, role).await?;
            tracing::info!("user_set_role: {summary}");
        } else {
            tracing::debug!("Creating user `{jid}`…");
            let summary = prosodyctl
                .user_create(jid.as_str(), &password, Some(role))
                .await?;
            tracing::info!("user_create: {summary}");
        };

        // Store the password in the secrets store for later use.
        secrets
            .set_password(
                jid.to_owned(),
                password.clone(),
                &mut passwords_guard,
                &mut tokens_guard,
            )
            .await?;

        // Create an authentication token to speed up future API calls
        // and avoid having thousands of tokens per service account.
        let token = oauth2
            .util_log_in(
                username,
                &password,
                &secrets.oauth2_client_credentials.load(),
            )
            .await?
            .access_token;
        #[cfg_attr(not(debug_assertions), allow(unused))]
        let previous_token = secrets
            .save_token(jid.to_owned(), token.clone().into(), &mut tokens_guard)
            .await;

        // NOTE: We just changed the password and hold a lock on tokens
        //   therefore we can be sure any previously stored token has been
        //   discarded already. For safety, here is a debug-only assertion.
        debug_assert!(
            previous_token.is_none(),
            "Token not discarded when changing password via `SecretService`."
        );

        // Create vCard if necessary.
        let creds = prosody_rest::CallerCredentials {
            bare_jid: jid.to_owned(),
            auth_token: token.clone(),
        };
        {
            tracing::debug!("Getting vCard for `{jid}`…");
            let vcard_opt = prosody_rest
                .get_own_vcard(&creds)
                .await
                .context(format!("Error getting vCard for `{jid}`"))?;
            if vcard_opt.is_none() {
                let vcard = VCard4 {
                    fn_: vec![vcard4::Fn_ {
                        value: name.clone(),
                    }],
                    // NOTE: See [XEP-0292: vCard4 Over XMPP](https://xmpp.org/extensions/xep-0292.html#apps)
                    //   and [RFC 6473: vCard KIND:application](https://www.rfc-editor.org/rfc/rfc6473.html).
                    kind: Some(vcard4::Kind::Application),
                    ..Default::default()
                };
                tracing::debug!("Creating vCard for `{jid}`…");
                prosody_rest
                    .set_own_vcard(vcard, &creds)
                    .await
                    .context(format!("Error creating vCard for `{jid}`"))?;
                tracing::info!("Created vCard for `{jid}`.");
            }
        }
    }

    let accounts_jids: Vec<BareJid> = accounts.into_iter().map(|(jid, _, _)| jid).collect();

    Ok(accounts_jids)
}

/// Creates the “Team” group for now, maybe more later.
async fn create_groups(
    prosodyctl: &mut Prosodyctl,
    groups: &Groups,
    server_domain: &JidDomain,
) -> Result<(), anyhow::Error> {
    let host: &str = server_domain.as_str();

    for (group_id, group_info) in groups.iter() {
        if !prosodyctl.groups_exists(host, group_id).await? {
            tracing::debug!("Creating group `{group_id}` on host `{host}`…");
            let summary = prosodyctl
                .groups_create(host, &group_info.name, None, Some(group_id))
                .await?;
            tracing::info!("groups_create: {summary}");
        }
    }

    Ok(())
}

/// Adds the “prose-workspace” user to the “Team” group for now, maybe more later.
///
/// NOTE: Adding the “prose-workspace” XMPP account to everyone’s rosters is
///   required for them to receive Workspace icon/accent color updates
///   (and all future PEP-based features).
async fn add_service_accounts_to_groups<'a, A, G>(
    prosodyctl: &mut Prosodyctl,
    service_accounts: A,
    group_ids: G,
    server_domain: &JidDomain,
) -> Result<(), anyhow::Error>
where
    A: Iterator<Item = JidNode>,
    G: Iterator<Item = &'a String> + Clone,
{
    let host: &str = server_domain.as_str();

    for ref username in service_accounts {
        for group_id in group_ids.clone() {
            tracing::debug!("Adding `{username}` to group `{group_id}`…");
            let summary = prosodyctl
                .groups_add_member(host, group_id, username, Some(true))
                .await?;
            tracing::info!("groups_add_member: {summary}");
        }
    }

    Ok(())
}

/// Synchronizes rosters (do group subscriptions).
/// This ensures all group members are correctly subscribed.
///
/// Note that when creating groups in a previous step, most groups might have
/// been skipped because they existed already. This means the automatic
/// `do_all_group_subscriptions_by_group` might not be triggered. Since we are
/// going to do the subscriptions here anyway, we used `delay_update` there.
///
/// NOTE: Rosters resynchronization (for groups) is an expensive operation
///   (`O(n^2)`), so it might take a while.
async fn synchronize_rosters<'a, G>(
    prosodyctl: &mut Prosodyctl,
    group_ids: G,
    server_domain: &JidDomain,
) -> Result<(), anyhow::Error>
where
    G: Iterator<Item = &'a String>,
{
    let host: &str = server_domain.as_str();

    for group_id in group_ids {
        tracing::debug!("Synchronizing groups…");
        let summary = prosodyctl.groups_sync(host, group_id).await?;
        tracing::info!("groups_sync: {summary}");
    }

    Ok(())
}

async fn run_migrations(
    app_config: &AppConfig,
    backend: &backend::Running,
) -> Result<(), anyhow::Error> {
    let prosody_data_dir = {
        let mut prosodyctl = backend.prosodyctl.write().await;
        prosodyctl.prosody_paths_data().await?
    };

    // Delete foundations of the previous architecture.
    {
        // Delete `admin.prose.local` data.
        let path = Path::new(&prosody_data_dir).join("admin%2eprose%2elocal");
        if path.is_dir() {
            tracing::warn!("Deleting <{path}>…", path = path.display());
            remove_dir_all(&path)
                .context(format!("Could not delete <{path}>", path = path.display()))?;
        } else {
            tracing::debug!(
                "Not deleting <{path}>: Not a directory.",
                path = path.display()
            );
        }
    }

    // Update Workspace vCard
    {
        use crate::router::workspace::{
            PatchWorkspaceCommand, patch_workspace_vcard_unchecked, service_account_credentials,
        };

        let ref jid = app_config.workspace_jid();
        let ref creds = service_account_credentials(backend, jid).await?;

        let dashboard_url = app_config.dashboard_url();
        let api_url = app_config.pod_api_url()?;

        patch_workspace_vcard_unchecked(
            backend,
            creds,
            PatchWorkspaceCommand {
                prose_pod_dashboard_url: Some(Some(dashboard_url.to_owned())),
                prose_pod_api_url: Some(Some(api_url)),
                auto_update_enabled: Some(app_config.policies.auto_update_enabled),
                ..Default::default()
            },
        )
        .await?;
    }

    Ok(())
}

// MARK: Data structures

#[derive(Debug)]
#[repr(transparent)]
struct Groups(HashMap<String, GroupInfo>);

#[derive(Debug)]
struct GroupInfo {
    name: String,
}

impl Groups {
    fn new(config: &crate::app_config::TeamsConfig) -> Self {
        let mut data: HashMap<String, GroupInfo> = HashMap::with_capacity(1);

        use crate::app_config::defaults;
        data.insert(
            defaults::MAIN_TEAM_GROUP_ID.to_owned(),
            GroupInfo {
                name: config.main_team_name.clone(),
            },
        );

        Self(data)
    }
}

// MARK: - Helpers

pub(crate) fn server_salt_path() -> PathBuf {
    Path::new(SERVER_DATA_DIR).join("salt.bin")
}

// MARK: - Boilerplate

impl std::ops::Deref for Groups {
    type Target = HashMap<String, GroupInfo>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
