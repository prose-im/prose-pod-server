// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! This example, in addition to show how this library can be used, ensures
//! that it supports the use case of migrating [Prose] Pods from their
//! [early 2026 architecture] to their [late 2026 architecture].
//!
//! [Prose]: https://prose.org/ "Prose IM homepage"
//! [early 2026 architecture]: https://github.com/prose-im/prose-pod-server/blob/b881891e442d35ad6bfdf65ec164cc6911855ba3/api/docs/ARCHITECTURE.md
//! [late 2026 architecture]: https://github.com/prose-im/prose-pod-api/discussions/368

mod common;
mod prose;

use std::sync::Arc;

use anyhow::Context as _;

use crate::common::lifecycle::{ExampleContext, keep_tmpdir};
use crate::common::util::*;
use crate::prose::dashboard::Dashboard;
use crate::prose::{init_prose_config, init_tsks};

/// Happy path of running the Prose Pod Dashboard to create, read and delete
/// backups.
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let mut context = common::lifecycle::init(&EXAMPLE_FS_TREE)?;

    let tmpdir = context.tmpdir();
    init_tsks(tmpdir.path()).context("Failed creating tsks")?;
    init_prose_config(tmpdir.path()).context("Failed creating prose.toml")?;
    drop(tmpdir);

    try_main(&mut context)
        .await
        .inspect_err(|_| keep_tmpdir(&context.tmpdir))
}

async fn try_main(context: &mut ExampleContext) -> Result<(), anyhow::Error> {
    let api = Arc::default();
    let dashboard = Dashboard::new(Arc::clone(&api));

    // Start v2 API.
    let api_v2 = prose::api::start_v2()?;
    *api.write().await = Some(Box::new(api_v2));

    // Create v2 backup.
    let backup_v2 = dashboard.create_backup("Example 1").await?;
    println!("Created backup: {}\n", backup_v2.display());

    // Register a cleanup function to delete backup and checks if an error happens.
    context.cleanup_functions.push(Box::pin({
        let backup_id = backup_v2.backup_id.clone();
        let api = Arc::clone(&api);

        async move {
            if let Some(api) = api.read().await.as_ref() {
                let backups = (api.get_backups().await)
                    .inspect_err(|err| tracing::error!("Failed listing backups: {err:#}"))
                    .unwrap_or_default();
                if backups.into_iter().any(|b| b.id.to_string() == backup_id) {
                    api.delete_backup(backup_id)
                        .await
                        .unwrap_or_else(|err| tracing::error!("Failed deleting backup: {err:#}"));
                }
            }
        }
    }));

    // Start v3 API.
    let api_v3 = prose::api::start_v3()?;
    *api.write().await = Some(Box::new(api_v3));

    // Modify some files to test that restoration works.
    override_files!([
        "etc/prose/prose.env",
        "var/lib/prose-pod-server/salt.bin",
        "var/lib/prose-pod-api/database.sqlite",
    ], in: context.tmpdir(), to: "bar");

    // Try to restore v2 backup from v3.
    () = dashboard
        .restore_backup(String::clone(&backup_v2.backup_id))
        .await?;
    println!();

    assert_file_contents!([
        "etc/prose/prose.env",
        "var/lib/prose/salt.bin",
        "var/lib/prose/database.sqlite",
    ], in: context.tmpdir(), eq: "foo");

    () = dashboard
        .delete_backup(String::clone(&backup_v2.backup_id))
        .await?;
    println!();

    Ok(())
}

/// The fake files to create at the start of this example.
///
/// Format: `("volume", "path", "contents")`.
#[rustfmt::skip]
const EXAMPLE_FS_TREE: [(&str, &str); 13] = [
    ("etc/prose/compose.yaml", ""),
    ("etc/prose/prose.env", "foo"),
    ("etc/prose/prose.lic", ""),
    ("etc/prose/prose.toml", ""),
    ("etc/prosody/prosody.cfg.lua", ""),
    ("var/lib/prose-pod-api/database.sqlite", "foo"),
    ("var/lib/prose-pod-server/salt.bin", "foo"),
    ("var/lib/prosody/example%2eorg/account_roles/john%2doe.dat", ""),
    ("var/lib/prosody/example%2eorg/accounts/john%2doe.dat", ""),
    ("var/lib/prosody/example%2eorg/auth_tokens/john%2doe.dat", ""),
    ("var/lib/prosody/example%2eorg/cron.dat", ""),
    ("var/lib/prosody/example%2eorg/group_info/team.dat", ""),
    ("var/lib/prosody/example%2eorg/groups/team.dat", ""),
];
