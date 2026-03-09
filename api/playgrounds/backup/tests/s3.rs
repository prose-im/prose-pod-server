// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[allow(dead_code, unused_imports)]
mod common;

use std::{collections::HashMap, path::PathBuf, time::Duration};

use anyhow::anyhow;
use prose_backup::{BackupConfig, BackupService};
use toml::toml;

use crate::common::*;

#[tokio::test]
async fn test_s3() -> Result<(), anyhow::Error> {
    let (_test_id, now) = init();

    let region = env_required!("S3_REGION");
    let endpoint_url = env_required!("S3_ENDPOINT_URL");
    let access_key = env_required!("S3_ACCESS_KEY");
    let secret_key = env_required!("S3_SECRET_KEY");
    let bucket_name_backups = env_required!("S3_BUCKET_NAME_BACKUPS");
    let bucket_name_checks = env_required!("S3_BUCKET_NAME_CHECKS");

    let backup_config = BackupConfig::try_from(toml! {
        [encryption]
        mode = "pgp"
        pgp.tsk = "encrypt.pgp"

        [signing]
        pgp.enabled = true
        pgp.tsk = "sign.pgp"

        [storage.backups]
        mode = "s3"
        s3.bucket_name = bucket_name_backups

        [storage.checks]
        mode = "s3"
        s3.bucket_name = bucket_name_checks

        [s3]
        region = region
        endpoint_url = endpoint_url
        access_key = access_key
        secret_key = secret_key
    })?;
    tracing::debug!("Parsed config: {backup_config:#?}");

    let certs: HashMap<PathBuf, openpgp::Cert> = make_test_certs([
        ("encrypt.pgp", now - Duration::from_hours(23)),
        ("sign.pgp", now - Duration::from_hours(23)),
    ])?;

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let service = BackupService::from_config_custom(
        backup_config,
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
    )?;

    service.list_backups().await?;

    todo!()
}
