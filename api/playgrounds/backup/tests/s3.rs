// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[allow(dead_code, unused_imports)]
mod common;

use std::{collections::HashMap, path::PathBuf, time::Duration};

use anyhow::{Context as _, anyhow};
use prose_backup::{
    BackupConfig, BackupService, CreateBackupCommand, CreateBackupSuccess,
    ExtractAndRestoreSuccess, archiving,
    config::{S3ObjectLockConfig, StorageS3Config},
    stats::print_stats,
    stores::{ObjectStore, S3Store},
};
use toml::toml;

use crate::common::{log_error, prelude::*};

#[tokio::test(flavor = "multi_thread")]
async fn test_s3_basic() -> Result<(), anyhow::Error> {
    let mut context = init();
    let TestContext {
        now,
        ref test_id,
        ref test_data_path,
        ..
    } = context;

    let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");
    let region = env_required!("S3_REGION");
    let endpoint_url = env_required!("S3_ENDPOINT_URL");
    let access_key = env_required!("S3_ACCESS_KEY");
    let secret_key = env_required!("S3_SECRET_KEY");
    let bucket_name_backups = env_required!("S3_BUCKET_NAME_BACKUPS");
    let bucket_name_checks = env_required!("S3_BUCKET_NAME_CHECKS");

    print!("\n");
    tracing::info!("Create config");
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

    print!("\n");
    tracing::info!("Create OpenPGP TSKs");
    let certs: HashMap<PathBuf, openpgp::Cert> = make_test_certs([
        ("encrypt.pgp", now - Duration::from_hours(23)),
        ("sign.pgp", now - Duration::from_hours(23)),
    ])?;

    // Create blueprints.
    let blueprints = test_blueprints();
    let archive_version = BLUEPRINT_POD_API_DEMO;
    let pod_api_demo_blueprint = blueprints.get(&archive_version).unwrap();
    let blueprint = pod_api_demo_blueprint
        .src_relative_to(format!("{prose_pod_api_dir}/local-run/scenarios/demo"));
    let restore_blueprint = pod_api_demo_blueprint.src_relative_to(test_data_path.join("restore"));

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    print!("\n");
    tracing::info!("Create service");
    let service = BackupService::from_config_custom(
        backup_config,
        archiving::Context { blueprints },
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
    )?;

    // Store some values for later use.
    let backup_store = as_s3_store(&service.backup_store);
    let check_store = as_s3_store(&service.check_store);

    print!("\n");
    tracing::info!("Create backup");
    let CreateBackupSuccess {
        creation_output,
        creation_stats,
    } = service
        .create_backup(CreateBackupCommand {
            prefix: &test_id,
            description: "Test backup",
            version: archive_version,
            blueprint: &blueprint,
            created_at: now,
        })
        .await?;
    let created_backup_id = creation_output.backup_id;
    tracing::info!("Upload stats: {creation_stats:#?}");

    // Register cleanup function.
    context.cleanup_functions.push({
        let backup_store = backup_store.clone();
        let check_store = check_store.clone();
        let created_backup_id = created_backup_id.clone();

        Box::pin(async move {
            (backup_store.delete(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());

            (check_store.delete_all(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());
        })
    });

    print!("\n");
    tracing::info!("List backups");
    let backups = service.list_backups().await?;
    tracing::debug!("Backups: {backups:#?}");
    assert!(backups.iter().any(|backup| backup.id == created_backup_id));

    print!("\n");
    tracing::info!("Get backup details");
    let details = service.get_details(&created_backup_id).await?;
    tracing::debug!("Backup details: {details:#?}");

    print!("\n");
    tracing::info!("Get download URL");
    let download_url = service
        .get_download_url(&created_backup_id, Duration::from_secs(3))
        .await?;
    tracing::debug!("Download URL: <{download_url}>.");

    print!("\n");
    tracing::info!("Restore backup");
    let ExtractAndRestoreSuccess {
        extraction_stats, ..
    } = service
        .restore_backup(&created_backup_id, &restore_blueprint)
        .await?;
    print_stats(
        &extraction_stats.raw_read_stats,
        &extraction_stats.decryption_stats,
        &extraction_stats.decompression_stats,
        extraction_stats.extracted_bytes_count,
    );

    print!("\n");
    tracing::info!("Delete backup");
    () = service.delete_backup(&created_backup_id).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_s3_object_locking() -> Result<(), anyhow::Error> {
    use s3::types::{
        ObjectLockConfiguration, ObjectLockEnabled, ObjectLockLegalHold, ObjectLockRetention,
    };

    let mut context = init();
    let TestContext {
        now, ref test_id, ..
    } = context;

    let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");
    let region = env_required!("S3_REGION");
    let endpoint_url = env_required!("S3_ENDPOINT_URL");
    let access_key = env_required!("S3_ACCESS_KEY");
    let secret_key = env_required!("S3_SECRET_KEY");
    let bucket_name_backups = env_required!("S3_BUCKET_NAME_BACKUPS");
    let bucket_name_checks = env_required!("S3_BUCKET_NAME_CHECKS");

    print!("\n");
    tracing::info!("Create config");
    let backup_config = BackupConfig::try_from(toml! {
        [storage.backups]
        mode = "s3"
        s3.bucket_name = bucket_name_backups

        [storage.checks]
        mode = "s3"
        s3.bucket_name = bucket_name_checks
        s3.object_lock_mode = "governance"
        s3.object_lock_duration = "PT5M"
        s3.object_lock_legal_hold_status = "on"

        [s3]
        region = region
        endpoint_url = endpoint_url
        access_key = access_key
        secret_key = secret_key
    })?;
    tracing::debug!("Parsed config: {backup_config:#?}");

    // Extract some parsed values for later use.
    let object_lock_mode = match &backup_config.storage.checks {
        prose_backup::config::StorageSubconfig::S3 { config } => {
            config.object_lock.as_ref().unwrap().mode.clone()
        }
        prose_backup::config::StorageSubconfig::Fs { .. } => unreachable!(),
    };
    let legal_hold_status = match &backup_config.storage.checks {
        prose_backup::config::StorageSubconfig::S3 { config } => {
            config.object_lock_legal_hold_status.clone().unwrap()
        }
        prose_backup::config::StorageSubconfig::Fs { .. } => unreachable!(),
    };

    // Create blueprints.
    let blueprints = test_blueprints();
    let archive_version = BLUEPRINT_POD_API_DEMO;
    let pod_api_demo_blueprint = blueprints.get(&archive_version).unwrap();
    let blueprint = pod_api_demo_blueprint
        .src_relative_to(format!("{prose_pod_api_dir}/local-run/scenarios/demo"));

    print!("\n");
    tracing::info!("Create service");
    let service = BackupService::from_config(backup_config, blueprints)?;

    // Store some values for later use.
    let backup_store = as_s3_store(&service.backup_store);
    let check_store = as_s3_store(&service.check_store);
    let ref s3_client = check_store.client;

    print!("\n");
    tracing::info!("Create backup");
    let CreateBackupSuccess {
        creation_output,
        creation_stats,
    } = service
        .create_backup(CreateBackupCommand {
            prefix: &test_id,
            description: "Test backup",
            version: archive_version,
            blueprint: &blueprint,
            created_at: now,
        })
        .await?;
    let created_backup_id = creation_output.backup_id;
    tracing::info!("Upload stats: {creation_stats:#?}");

    // Register cleanup function.
    context.cleanup_functions.push({
        let backup_store = backup_store.clone();
        let check_store = check_store.clone();
        let created_backup_id = created_backup_id.clone();

        Box::pin(async move {
            (backup_store.delete(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());

            (check_store.delete_all(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());
        })
    });

    print!("\n");
    {
        let digest_id = creation_output.digest_ids.first().unwrap().to_owned();

        // Bucket lock config
        let lock_config = s3_client
            .get_object_lock_configuration()
            .bucket(&check_store.bucket)
            .send()
            .await?;
        assert_eq!(
            lock_config
                .object_lock_configuration()
                .map(ObjectLockConfiguration::object_lock_enabled)
                .flatten(),
            Some(&ObjectLockEnabled::Enabled),
            "lock_config: {lock_config:#?}"
        );

        // Object retention
        let retention = s3_client
            .get_object_retention()
            .bucket(&check_store.bucket)
            .key(digest_id.to_string())
            .send()
            .await?;
        assert_eq!(
            retention
                .retention()
                .map(ObjectLockRetention::mode)
                .flatten(),
            Some(&object_lock_mode),
            "retention: {retention:#?}"
        );

        // Legal hold
        let legal_hold = s3_client
            .get_object_legal_hold()
            .bucket(&check_store.bucket)
            .key(digest_id.to_string())
            .send()
            .await?;
        assert_eq!(
            legal_hold
                .legal_hold()
                .map(ObjectLockLegalHold::status)
                .flatten(),
            Some(&legal_hold_status),
            "legal_hold: {legal_hold:#?}"
        );

        // Try to delete an integrity check.
        {
            // NOTE: Does not error because a delete marker is created but the
            //   underlying object is kept per the Object Lock configuration.
            let _deleted_state = service.check_store.delete(&digest_id).await?;
            // FIXME: Re-enable this assertion? Seems to fail with Ceph.
            // assert_eq!(deleted_state, DeletedState::MarkedForDeletion);

            let versions = s3_client
                .list_object_versions()
                .bucket(&check_store.bucket)
                .prefix(digest_id.to_string())
                .send()
                .await?;
            assert!(
                !versions.delete_markers().is_empty(),
                "versions={versions:#?}"
            );
        }
    }

    // crate::common::s3::print_all_objects(s3_client, &check_store.bucket).await?;

    Ok(())
}

/// Test Object Lock via one-shot upload.
///
/// TL;DR: When using Ceph, Object Lock modes and Legal Hold statuses are
///   respected when sending “Put Object” requests.
#[tokio::test(flavor = "multi_thread")]
async fn test_object_lock_oneshot() -> Result<(), anyhow::Error> {
    use s3::types::{
        ObjectLockLegalHold, ObjectLockLegalHoldStatus, ObjectLockMode, ObjectLockRetention,
        ObjectLockRetentionMode,
    };

    let mut context = init();
    let TestContext {
        now, ref test_id, ..
    } = context;

    let s3_store = test_s3_store(None, None)?;
    let ref s3_client = s3_store.client;

    let key = format!("{test_id}-lock-oneshot");

    let (object_lock_mode, object_lock_retention) = (
        ObjectLockMode::Compliance,
        ObjectLockRetentionMode::Compliance,
    );
    let object_lock_legal_hold_status = ObjectLockLegalHoldStatus::On;

    s3_client
        .put_object()
        .bucket(&s3_store.bucket)
        .key(&key)
        .body(bytes::Bytes::from("test").into())
        .object_lock_mode(object_lock_mode)
        .object_lock_retain_until_date((now + Duration::from_mins(1)).into())
        .object_lock_legal_hold_status(object_lock_legal_hold_status.clone())
        .send()
        .await?;

    // Register cleanup function.
    context.cleanup_functions.push({
        let s3_store = s3_store.clone();
        let key = key.clone();

        Box::pin(async move {
            match s3_store.delete(&key).await {
                Ok(_) => {}
                Err(err) => tracing::error!("{err:?}"),
            }
        })
    });

    let object = s3_client
        .get_object()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await?;
    let object_bytes = object.body.collect().await.unwrap();
    assert_eq!(object_bytes.to_vec().len(), 4);

    let retention = s3_client
        .get_object_retention()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await?;
    assert_eq!(
        retention
            .retention()
            .map(ObjectLockRetention::mode)
            .flatten(),
        Some(&object_lock_retention),
        "retention: {retention:#?}"
    );

    let legal_hold = s3_client
        .get_object_legal_hold()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await?;
    assert_eq!(
        legal_hold
            .legal_hold()
            .map(ObjectLockLegalHold::status)
            .flatten(),
        Some(&object_lock_legal_hold_status),
        "legal_hold: {legal_hold:#?}"
    );

    Ok(())
}

/// Test Object Lock via multipart upload.
///
/// TL;DR: When using Ceph, Object Lock modes and Legal Hold statuses are NOT
///   respected when sending “Multipart Upload” requests. One needs to apply
///   this metadata afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn test_object_lock_multipart() -> Result<(), anyhow::Error> {
    use s3::error::SdkError;
    use s3::types::{
        CompletedMultipartUpload, CompletedPart, ObjectLockLegalHold, ObjectLockLegalHoldStatus,
        ObjectLockMode, ObjectLockRetention, ObjectLockRetentionMode,
    };

    let mut context = init();
    let TestContext {
        now, ref test_id, ..
    } = context;

    let s3_store = test_s3_store(
        Some(S3ObjectLockConfig {
            mode: ObjectLockRetentionMode::Governance,
            duration: Duration::from_mins(5),
        }),
        Some(ObjectLockLegalHoldStatus::On),
    )?;
    let ref s3_client = s3_store.client;

    let key = format!("{test_id}-lock-multipart");

    // Initiate multipart upload
    let multipart = s3_client
        .create_multipart_upload()
        .bucket(&s3_store.bucket)
        .key(&key)
        .object_lock_mode(ObjectLockMode::Compliance)
        .object_lock_retain_until_date((now + Duration::from_mins(2)).into())
        .object_lock_legal_hold_status(ObjectLockLegalHoldStatus::On)
        .send()
        .await?;

    let upload_id = multipart.upload_id().unwrap();

    // Upload single part.
    let body = bytes::Bytes::from("test");
    let upload_part = s3_client
        .upload_part()
        .bucket(&s3_store.bucket)
        .key(&key)
        .upload_id(upload_id)
        .part_number(1)
        .body(body.into())
        .send()
        .await?;

    // Complete the multipart upload
    let completed_part = CompletedPart::builder()
        .part_number(1)
        .e_tag(upload_part.e_tag().unwrap())
        .build();
    let completed_upload = CompletedMultipartUpload::builder()
        .parts(completed_part)
        .build();
    s3_client
        .complete_multipart_upload()
        .bucket(&s3_store.bucket)
        .key(&key)
        .upload_id(upload_id)
        .multipart_upload(completed_upload)
        .send()
        .await?;

    // Register cleanup function.
    context.cleanup_functions.push({
        let s3_store = s3_store.clone();
        let key = key.clone();

        Box::pin(async move {
            match s3_store.delete(&key).await {
                Ok(_) => {}
                Err(err) => tracing::error!("{err:?}"),
            }
        })
    });

    let object = s3_client
        .get_object()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await?;
    let object_bytes = object.body.collect().await.unwrap();
    assert_eq!(object_bytes.to_vec().len(), 4);

    // After a multipart upload, `object_retention` is incorrect.
    let retention = s3_client
        .get_object_retention()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await;
    let retention_error_code = match &retention {
        Err(SdkError::ServiceError(error)) => error.err().meta().code(),
        Err(_) | Ok(_) => None,
    };
    assert_eq!(
        retention_error_code,
        Some("ObjectLockConfigurationNotFoundError"),
        "retention: {retention:#?}"
    );

    // After a multipart upload, `object_legal_hold` is incorrect.
    let legal_hold = s3_client
        .get_object_legal_hold()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await;
    let legal_hold_error_code = match &legal_hold {
        Err(SdkError::ServiceError(error)) => error.err().meta().code(),
        Err(_) | Ok(_) => None,
    };
    assert_eq!(
        legal_hold_error_code,
        Some("ObjectLockConfigurationNotFoundError"),
        "legal_hold: {legal_hold:#?}"
    );

    // Manually set `object_retention`.
    s3_client
        .put_object_retention()
        .bucket(&s3_store.bucket)
        .key(&key)
        // .version_id(version_id)
        .retention(
            ObjectLockRetention::builder()
                .mode(ObjectLockRetentionMode::Compliance)
                .retain_until_date((now + Duration::from_mins(2)).into())
                .build(),
        )
        .send()
        .await
        .context("Failed setting S3 object retention")?;

    // Now `object_retention` is correct.
    let retention = s3_client
        .get_object_retention()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await;
    assert!(retention.is_ok(), "retention: {retention:#?}");

    // Manually set `object_legal_hold`.
    s3_client
        .put_object_legal_hold()
        .bucket(&s3_store.bucket)
        .key(&key)
        // .version_id(version_id)
        .legal_hold(
            ObjectLockLegalHold::builder()
                .status(ObjectLockLegalHoldStatus::On)
                .build(),
        )
        .send()
        .await
        .context("Failed setting S3 legal hold")?;

    // Now `object_legal_hold` is correct.
    let legal_hold = s3_client
        .get_object_legal_hold()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await;
    assert!(legal_hold.is_ok(), "legal_hold: {legal_hold:#?}");

    Ok(())
}

// MARK: Helpers

fn test_s3_store(
    object_lock: Option<S3ObjectLockConfig>,
    object_lock_legal_hold_status: Option<s3::types::ObjectLockLegalHoldStatus>,
) -> Result<S3Store, anyhow::Error> {
    let region = env_required!("S3_REGION");
    let endpoint_url = env_required!("S3_ENDPOINT_URL");
    let access_key = env_required!("S3_ACCESS_KEY");
    let secret_key = env_required!("S3_SECRET_KEY");
    let bucket_name_checks = env_required!("S3_BUCKET_NAME_CHECKS");

    Ok(S3Store::from_config(&StorageS3Config {
        bucket_name: bucket_name_checks,
        region,
        endpoint_url,
        access_key,
        secret_key: secret_key.into(),
        session_token: None,
        force_path_style: None,
        object_lock,
        object_lock_legal_hold_status,
    }))
}

fn as_s3_store<'a>(object_store: &'a Box<dyn ObjectStore>) -> &'static S3Store {
    unsafe {
        &*(object_store.as_ref() as *const dyn prose_backup::stores::ObjectStore
            as *const prose_backup::stores::S3)
    }
}
