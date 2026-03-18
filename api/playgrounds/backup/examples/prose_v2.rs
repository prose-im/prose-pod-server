// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! This example, in addition to show how this library can be used, ensures
//! that it supports the use case of [Prose] Pods, in their
//! [early 2026 architecture].
//!
//! [Prose]: https://prose.org/ "Prose IM homepage"
//! [early 2026 architecture]: https://github.com/prose-im/prose-pod-server/blob/b881891e442d35ad6bfdf65ec164cc6911855ba3/api/docs/ARCHITECTURE.md

mod common;
mod prose;

use std::sync::Arc;

use anyhow::Context as _;
use tokio::sync::RwLock;

use crate::common::lifecycle::{ExampleContext, keep_tmpdir};
use crate::prose::api::ProsePodApi;
use crate::prose::dashboard::Dashboard;

/// Happy path of running the Prose Pod Dashboard to create, read and delete
/// backups.
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let context = common::lifecycle::init(&EXAMPLE_FS_TREE)?;

    try_main(&context)
        .await
        .inspect_err(|_| keep_tmpdir(&context.tmpdir))
}

async fn try_main(context: &ExampleContext) -> Result<(), anyhow::Error> {
    let api = prose::api::start_v2()?;
    let api: Arc<RwLock<Option<Box<dyn ProsePodApi>>>> = Arc::new(RwLock::new(Some(Box::new(api))));

    let dashboard = Dashboard::new(Arc::clone(&api));

    let backups = dashboard.show_backups().await?;
    let backup_count_before = backups.len();

    let backup_details = dashboard.create_backup("Example 1").await?;
    debug_assert_eq!(backup_details.description.as_str(), "Example 1");

    // TODO: Register cleanup function.

    let backups = dashboard.show_backups().await?;
    debug_assert_eq!(backups.len(), backup_count_before + 1);

    let backup_id = &backups[0].backup_id;

    let details = dashboard.inspect_backup(String::clone(&backup_id)).await?;
    debug_assert!(details.is_encrypted);

    let _download_url = dashboard.download_backup(String::clone(&backup_id)).await?;
    // Manually test that the URL works (it does).
    // tracing::info!("Download URL: {download_url}");
    // crate::common::util::press_enter_to_continue();

    // Modify some files to test that restoration works.
    override_files!([
        "etc/prose/prose.env",
        "var/lib/prose-pod-api/database.sqlite",
    ], in: context.tmpdir(), to: "bar");

    assert_file_contents!([
        "etc/prose/prose.env",
        "var/lib/prose-pod-api/database.sqlite",
    ], in: context.tmpdir(), eq: "bar");

    () = dashboard.restore_backup(String::clone(&backup_id)).await?;

    assert_file_contents!([
        "etc/prose/prose.env",
        "var/lib/prose-pod-api/database.sqlite",
    ], in: context.tmpdir(), eq: "foo");

    () = dashboard.delete_backup(String::clone(&backup_id)).await?;

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
    ("var/lib/prose-pod-server/salt.bin", ""),
    ("var/lib/prosody/example%2eorg/account_roles/john%2doe.dat", ""),
    ("var/lib/prosody/example%2eorg/accounts/john%2doe.dat", ""),
    ("var/lib/prosody/example%2eorg/auth_tokens/john%2doe.dat", ""),
    ("var/lib/prosody/example%2eorg/cron.dat", ""),
    ("var/lib/prosody/example%2eorg/group_info/team.dat", ""),
    ("var/lib/prosody/example%2eorg/groups/team.dat", ""),
];

macro_rules! override_files {
    ($paths:expr, in: $tmpdir:expr, to: $contents:literal) => {
        for path in $paths.iter() {
            let path = $tmpdir.path().join(path);
            std::fs::write(&path, $contents).context(format!("Failed writing in {path:?}"))?;
        }
    };
}
pub(crate) use override_files;

macro_rules! assert_file_contents {
    ($paths:expr, in: $tmpdir:expr, eq: $contents:literal) => {
        for path in $paths.iter() {
            let path = $tmpdir.path().join(path);
            let env = std::fs::read_to_string(&path).context(format!("Failed reading {path:?}"))?;
            assert_eq!(env.as_str(), $contents);
        }
    };
}
pub(crate) use assert_file_contents;
