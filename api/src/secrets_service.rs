// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, anyhow};
use arc_swap::ArcSwap;
use prosody_http::{mod_http_oauth2::ProsodyOAuth2, oauth2};
use tokio::sync::{RwLockReadGuard, RwLockWriteGuard};
use tokio_util::sync::CancellationToken;

use crate::{
    models::{AuthToken, BareJid, Password},
    secrets_store::SecretsStore,
    util::{Cache, OptionRwLockReadGuard, debug_panic_or_log_error},
};

/// The things that manages service account secrets, and ensures that
/// at any time one can use service account tokens without having to
/// worry about account creation, password rotation or token expiry.
#[derive(Debug)]
pub struct SecretsService {
    pub store: SecretsStore,
    pub oauth2: Arc<ProsodyOAuth2>,
    pub oauth2_client_credentials: ArcSwap<oauth2::ClientCredentials>,
}

impl SecretsService {
    /// Write lock on passwords.
    ///
    /// This method doesn’t return a guard that can be used to access
    /// or mutate data, but rather something that can be used to lock
    /// access to passwords during batch writes.
    pub async fn passwords_rw_guard(&self) -> PasswordsWriteGuard<'_> {
        PasswordsWriteGuard {
            guard: self.store.passwords.write().await,
        }
    }

    /// Write lock on tokens cache.
    ///
    /// This method doesn’t return a guard that can be used to access
    /// or mutate data, but rather something that can be used to lock
    /// access to tokens during batch writes.
    pub async fn tokens_rw_guard(&self) -> TokensWriteGuard<'_> {
        TokensWriteGuard {
            guard: self.store.tokens_cache.write().await,
        }
    }

    pub async fn get_password_opt(
        &self,
        jid: &BareJid,
    ) -> OptionRwLockReadGuard<'_, HashMap<BareJid, Password>, Password> {
        let ref store = self.store.passwords;

        OptionRwLockReadGuard::map(store.read().await, |passwords| passwords.get(jid).cloned())
    }

    pub async fn get_password(
        &self,
        jid: &BareJid,
    ) -> Result<RwLockReadGuard<'_, Password>, anyhow::Error> {
        let ref store = self.store.passwords;

        if store.read().await.contains_key(jid) {
            // NOTE: While it might seem inefficient to create two
            //   read guards in a row and do unwrapping instead of
            //   pattern matching an optional value, Rust’s borrow
            //   checker doesn’t seem to provide a way around it.
            Ok(RwLockReadGuard::map(store.read().await, |passwords| {
                passwords.get(jid).unwrap()
            }))
        } else {
            Err(anyhow!("No password stored for `{jid}`"))
        }
    }

    /// NOTE: Invalidates the current auth token for that JID, if any.
    pub async fn set_password(
        &self,
        jid: BareJid,
        password: Password,
        passwords_guard: &mut PasswordsWriteGuard<'_>,
        tokens_guard: &mut TokensWriteGuard<'_>,
    ) -> Result<(), anyhow::Error> {
        let previous_token = tokens_guard.guard.remove(&jid);

        // Revoke previous token to avoid stranding.
        if let Some(previous_token) = previous_token {
            self.oauth2
                .revoke(&previous_token)
                .await
                .context("Could not revoke auth token after password update")?;
        }

        passwords_guard.guard.insert(jid, password);

        Ok(())
    }

    pub async fn get_token(
        &self,
        jid: &BareJid,
    ) -> Result<RwLockReadGuard<'_, AuthToken>, anyhow::Error> {
        let ref cache = self.store.tokens_cache;

        if !cache.read().await.contains_key(jid) {
            let username = jid.node().context("JID must contain a localpart")?;
            let password = self.get_password(jid).await?;

            let token = self
                .oauth2
                .util_log_in(username, &password, &self.oauth2_client_credentials.load())
                .await?
                .access_token;

            cache.write().await.insert(jid.clone(), token.into());

            // Drop read guard on passwords only after the token
            // has been written to prevent race conditions (e.g.
            // password being rotated while this is happening).
            drop(password);
        }

        // NOTE: While it might seem inefficient to create two
        //   read guards in a row and do unwrapping instead of
        //   pattern matching an optional value, Rust’s borrow
        //   checker doesn’t seem to provide a way around it.
        Ok(RwLockReadGuard::map(cache.read().await, |tokens| {
            tokens.get(jid).unwrap()
        }))
    }

    pub async fn save_token(
        &self,
        jid: BareJid,
        token: AuthToken,
        tokens_guard: &mut TokensWriteGuard<'_>,
    ) -> Option<AuthToken> {
        tokens_guard.guard.insert(jid, token)
    }

    pub fn run_purge_tasks(
        &self,
        cancellation_token: CancellationToken,
    ) -> impl Future<Output = ()> + 'static {
        let cache = self.store.tokens_cache.clone();
        async move {
            tokio::select! {
                () = Cache::purge_task(cache) => {
                    debug_panic_or_log_error!("Cache purge task ended.");
                }
                () = cancellation_token.cancelled_owned() => {
                    tracing::debug!("Cache purge task cancelled.");
                }
            }
        }
    }
}

pub struct PasswordsWriteGuard<'a> {
    guard: RwLockWriteGuard<'a, HashMap<BareJid, Password>>,
}

pub struct TokensWriteGuard<'a> {
    guard: RwLockWriteGuard<'a, Cache<BareJid, AuthToken>>,
}
