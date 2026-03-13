// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! TODO: Describe.

mod common;
mod prose;

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::prose::api::Api;
use crate::prose::dashboard::Dashboard;

/// Happy path of running the Prose Pod Dashboard to create, read and delete
/// backups.
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    common::lifecycle::init();

    let api = Api::start_v1()?;
    let api = Arc::new(RwLock::new(Some(api)));

    let dashboard = Dashboard::new(Arc::clone(&api));

    let backups = dashboard.show_backups().await?;
    debug_assert_eq!(backups.len(), 0);

    let backup_details = dashboard.create_backup("Example 1").await?;
    debug_assert_eq!(backup_details.description.as_str(), "Example 1");

    let backups = dashboard.show_backups().await?;
    debug_assert_eq!(backups.len(), 1);

    let ref backup_id = backups[0].backup_id;

    let details = dashboard.inspect_backup(String::clone(&backup_id)).await?;
    debug_assert!(details.is_encrypted);

    let _download_url = dashboard.download_backup(String::clone(&backup_id)).await?;
    // TODO: Test that the URL works.

    todo!()
}
