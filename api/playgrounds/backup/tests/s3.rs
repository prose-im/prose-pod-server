// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod common;

use prose_backup::{
    config::{S3ObjectLockConfig, StorageS3Config},
    stores::{ObjectId, ObjectStore, S3Store},
};

use crate::common::{prelude::*, print::print_stats};

#[tokio::test(flavor = "multi_thread")]
async fn s3_happy_path() {
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

    println!();
    tracing::info!("Create config");
    let backup_config = BackupConfig::try_from(toml! {
        [encryption]
        mode = "pgp"
        pgp.tsk = "encrypt.pgp"

        [signing]
        pgp.enabled = true
        pgp.tsk = "sign.pgp"

        [storage.backups]
        provider = "s3"
        s3.bucket_name = bucket_name_backups

        [storage.checks]
        provider = "s3"
        s3.bucket_name = bucket_name_checks

        [s3]
        region = region
        endpoint_url = endpoint_url
        access_key = access_key
        secret_key = secret_key
    })
    .unwrap();
    tracing::debug!("Parsed config: {backup_config:#?}");

    println!();
    tracing::info!("Create OpenPGP TSKs");
    let certs: HashMap<PathBuf, openpgp::Cert> = make_test_certs([
        ("encrypt.pgp", now - Duration::from_hours(23)),
        ("sign.pgp", now - Duration::from_hours(23)),
    ])
    .unwrap();

    // Create blueprints.
    let pod_api_demo_blueprint =
        ArchiveBlueprint::from_iter(BLUEPRINT_PATHS_POD_API_DEMO.into_iter());
    let blueprint = pod_api_demo_blueprint
        .src_relative_to(format!("{prose_pod_api_dir}/local-run/scenarios/demo"));
    let restore_blueprint = pod_api_demo_blueprint.src_relative_to(test_data_path.join("restore"));

    let backup_version: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(backup_version, blueprint.clone())
        .build();

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    println!();
    tracing::info!("Create service");
    let service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
    )
    .unwrap();

    // Store some values for later use.
    let backup_store = as_s3_store(&service.backup_store.inner());
    let check_store = as_s3_store(&service.check_store);

    println!();
    tracing::info!("Create backup");
    let CreateBackupSuccess {
        output: creation_output,
        stats: creation_stats,
        ..
    } = service
        .create_backup(CreateBackupCommand {
            prefix: &test_id,
            description: "Test backup",
            version: backup_version,
            blueprint: &blueprint,
            additional_archive_data: vec![],
            created_at: now,
        })
        .await
        .unwrap();
    let created_backup_id = creation_output.backup_id;
    tracing::info!("Upload stats: {creation_stats:#?}");

    // Register cleanup function.
    context.cleanup_functions.push({
        let backup_store = backup_store.clone();
        let check_store = check_store.clone();
        let created_backup_id = ObjectId::from(&created_backup_id);

        Box::pin(async move {
            (backup_store.delete(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());

            (check_store.delete_all(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());
        })
    });

    println!();
    tracing::info!("List backups");
    let backups = service.list_backups().await.unwrap();
    tracing::debug!("Backups: {backups:#?}");
    assert!(backups.iter().any(|backup| backup.id == created_backup_id));

    println!();
    tracing::info!("Get backup details");
    let details = service.get_details(&created_backup_id).await.unwrap();
    tracing::debug!("Backup details: {details:#?}");

    println!();
    tracing::info!("Get download URL");
    let download_url = service
        .get_download_url(&created_backup_id, Duration::from_secs(3))
        .await
        .unwrap();
    tracing::debug!("Download URL: <{download_url}>.");

    println!();
    tracing::info!("Restore backup");
    let ExtractAndRestoreSuccess {
        extraction_stats, ..
    } = service
        .restore_backup(&created_backup_id, &restore_blueprint)
        .await
        .unwrap();
    print_stats(
        &extraction_stats.raw_read_stats,
        &extraction_stats.decryption_stats,
        &extraction_stats.decompression_stats,
        extraction_stats.extracted_bytes_count,
    );

    println!();
    tracing::info!("Delete backup");
    () = service.delete_backup(&created_backup_id).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_single_bucket_same_prefix() {
    let mut context = init();
    let TestContext {
        now,
        ref test_id,
        ref test_data_path,
        ..
    } = context;

    let region = env_required!("S3_REGION");
    let endpoint_url = env_required!("S3_ENDPOINT_URL");
    let access_key = env_required!("S3_ACCESS_KEY");
    let secret_key = env_required!("S3_SECRET_KEY");
    let bucket_name_backups = env_required!("S3_BUCKET_NAME_BACKUPS");

    println!();
    tracing::info!("Create config");
    let backup_config = BackupConfig::try_from(toml! {
        [storage]
        provider = "s3"
        s3.prefix = "single-store/"

        [s3]
        bucket_name = bucket_name_backups
        region = region
        endpoint_url = endpoint_url
        access_key = access_key
        secret_key = secret_key
    })
    .unwrap();
    tracing::debug!("Parsed config: {backup_config:#?}");

    // Create blueprints.
    let blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
        .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let backup_version: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(backup_version, blueprint.clone())
        .build();

    println!();
    tracing::info!("Create service");
    let service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        |_| unreachable!(),
        || -> openpgp::policy::StandardPolicy { unreachable!() },
    )
    .unwrap();

    // Store some values for later use.
    let backup_store = as_s3_store(&service.backup_store.inner());
    let check_store = as_s3_store(&service.check_store);

    println!();
    tracing::info!("Create backup");
    let CreateBackupSuccess {
        output: creation_output,
        stats: creation_stats,
        ..
    } = service
        .create_backup(CreateBackupCommand {
            prefix: &test_id,
            description: "Test backup",
            version: backup_version,
            blueprint: &blueprint,
            additional_archive_data: vec![],
            created_at: now,
        })
        .await
        .unwrap();
    let created_backup_id = creation_output.backup_id;
    tracing::info!("Upload stats: {creation_stats:#?}");

    // Register cleanup function.
    context.cleanup_functions.push({
        let backup_store = backup_store.clone();
        let check_store = check_store.clone();
        let created_backup_id = ObjectId::from(&created_backup_id);

        Box::pin(async move {
            (backup_store.delete(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());

            (check_store.delete_all(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());
        })
    });

    println!();
    tracing::info!("List backups");
    let backups = service.list_backups().await.unwrap();
    tracing::debug!("Backups: {backups:#?}");
    assert!(backups.iter().any(|backup| backup.id == created_backup_id));
    assert!(
        backups
            .iter()
            .all(|backup| backup.id.extensions.ends_with(&[Box::from("zst")]))
    );

    println!();
    tracing::info!("Get backup details");
    let details = service.get_details(&created_backup_id).await.unwrap();
    tracing::debug!("Backup details: {details:#?}");

    println!();
    tracing::info!("Get download URL");
    let download_url = service
        .get_download_url(&created_backup_id, Duration::from_secs(3))
        .await
        .unwrap();
    tracing::debug!("Download URL: <{download_url}>.");

    println!();
    tracing::info!("Restore backup");
    let ExtractAndRestoreSuccess {
        extraction_stats, ..
    } = service
        .restore_backup(&created_backup_id, &blueprint)
        .await
        .unwrap();
    print_stats(
        &extraction_stats.raw_read_stats,
        &extraction_stats.decryption_stats,
        &extraction_stats.decompression_stats,
        extraction_stats.extracted_bytes_count,
    );

    println!();
    tracing::info!("Delete backup");
    () = service.delete_backup(&created_backup_id).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_object_locking() {
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

    println!();
    tracing::info!("Create config");
    let backup_config = BackupConfig::try_from(toml! {
        [storage.backups]
        provider = "s3"
        s3.bucket_name = bucket_name_backups

        [storage.checks]
        provider = "s3"
        s3.bucket_name = bucket_name_checks
        s3.object_lock_mode = "governance"
        s3.object_lock_duration = "PT5M"
        s3.object_lock_legal_hold_status = "on"

        [s3]
        region = region
        endpoint_url = endpoint_url
        access_key = access_key
        secret_key = secret_key
    })
    .unwrap();
    tracing::debug!("Parsed config: {backup_config:#?}");

    // Extract some parsed values for later use.
    let object_lock_mode = match &backup_config.storage.checks {
        prose_backup::config::StorageSubconfig::S3 { config } => {
            config.object_lock.as_ref().unwrap().mode.clone()
        }
        #[allow(unreachable_patterns)]
        _ => unreachable!(),
    };
    let legal_hold_status = match &backup_config.storage.checks {
        prose_backup::config::StorageSubconfig::S3 { config } => {
            config.object_lock_legal_hold_status.clone().unwrap()
        }
        #[allow(unreachable_patterns)]
        _ => unreachable!(),
    };

    // Create blueprints.
    let pod_api_demo_blueprint =
        ArchiveBlueprint::from_iter(BLUEPRINT_PATHS_POD_API_DEMO.into_iter());
    let blueprint = pod_api_demo_blueprint
        .src_relative_to(format!("{prose_pod_api_dir}/local-run/scenarios/demo"));

    println!();
    tracing::info!("Create service");
    let service = BackupService::from_config(&backup_config, HashMap::new()).unwrap();

    // Store some values for later use.
    let backup_store = as_s3_store(&service.backup_store.inner());
    let check_store = as_s3_store(&service.check_store);
    let s3_client = &check_store.client;

    println!();
    tracing::info!("Create backup");
    let CreateBackupSuccess {
        output: creation_output,
        stats: creation_stats,
        ..
    } = service
        .create_backup(CreateBackupCommand {
            prefix: &test_id,
            description: "Test backup",
            version: 0,
            blueprint: &blueprint,
            additional_archive_data: vec![],
            created_at: now,
        })
        .await
        .unwrap();
    let created_backup_id = creation_output.backup_id;
    tracing::info!("Upload stats: {creation_stats:#?}");

    // Register cleanup function.
    context.cleanup_functions.push({
        let backup_store = backup_store.clone();
        let check_store = check_store.clone();
        let created_backup_id = ObjectId::from(&created_backup_id);

        Box::pin(async move {
            (backup_store.delete(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());

            (check_store.delete_all(&created_backup_id))
                .await
                .map_or_else(log_error!(), |_| ());
        })
    });

    println!();
    {
        let digest_id = creation_output.digest_ids.first().unwrap().to_owned();

        // Bucket lock config
        let lock_config = s3_client
            .get_object_lock_configuration()
            .bucket(&check_store.bucket)
            .send()
            .await
            .unwrap();
        assert_eq!(
            lock_config
                .object_lock_configuration()
                .and_then(ObjectLockConfiguration::object_lock_enabled),
            Some(&ObjectLockEnabled::Enabled),
            "lock_config: {lock_config:#?}"
        );

        // Object retention
        let retention = s3_client
            .get_object_retention()
            .bucket(&check_store.bucket)
            .key(digest_id.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(
            retention.retention().and_then(ObjectLockRetention::mode),
            Some(&object_lock_mode),
            "retention: {retention:#?}"
        );

        // Legal hold
        let legal_hold = s3_client
            .get_object_legal_hold()
            .bucket(&check_store.bucket)
            .key(digest_id.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(
            legal_hold
                .legal_hold()
                .and_then(ObjectLockLegalHold::status),
            Some(&legal_hold_status),
            "legal_hold: {legal_hold:#?}"
        );

        // Try to delete an integrity check.
        {
            // NOTE: Does not error because a delete marker is created but the
            //   underlying object is kept per the Object Lock configuration.
            let _deleted_state = service.check_store.delete(&digest_id).await.unwrap();
            // FIXME: Re-enable this assertion? Seems to fail with Ceph.
            // assert_eq!(deleted_state, DeletedState::MarkedForDeletion);

            let versions = s3_client
                .list_object_versions()
                .bucket(&check_store.bucket)
                .prefix(digest_id.to_string())
                .send()
                .await
                .unwrap();
            assert!(
                !versions.delete_markers().is_empty(),
                "versions={versions:#?}"
            );
        }
    }

    // crate::common::s3::print_all_objects(s3_client, &check_store.bucket).await.unwrap();
}

/// Test Object Lock via one-shot upload.
///
/// TL;DR: When using Ceph, Object Lock modes and Legal Hold statuses are
///   respected when sending “Put Object” requests.
#[tokio::test(flavor = "multi_thread")]
async fn s3_object_lock_oneshot() {
    use s3::types::{
        ObjectLockLegalHold, ObjectLockLegalHoldStatus, ObjectLockMode, ObjectLockRetention,
        ObjectLockRetentionMode,
    };

    let mut context = init();
    let TestContext {
        now, ref test_id, ..
    } = context;

    let s3_store = test_s3_store(None, None).unwrap();
    let s3_client = &s3_store.client;

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
        .await
        .unwrap();

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
        .await
        .unwrap();
    let object_bytes = object.body.collect().await.unwrap();
    assert_eq!(object_bytes.to_vec().len(), 4);

    let retention = s3_client
        .get_object_retention()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(
        retention.retention().and_then(ObjectLockRetention::mode),
        Some(&object_lock_retention),
        "retention: {retention:#?}"
    );

    let legal_hold = s3_client
        .get_object_legal_hold()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(
        legal_hold
            .legal_hold()
            .and_then(ObjectLockLegalHold::status),
        Some(&object_lock_legal_hold_status),
        "legal_hold: {legal_hold:#?}"
    );
}

/// Test Object Lock via multipart upload.
///
/// TL;DR: When using Ceph, Object Lock modes and Legal Hold statuses are NOT
///   respected when sending “Multipart Upload” requests. One needs to apply
///   this metadata afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn s3_object_lock_multipart() {
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
    )
    .unwrap();
    let s3_client = &s3_store.client;

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
        .await
        .unwrap();

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
        .await
        .unwrap();

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
        .await
        .unwrap();

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
        .await
        .unwrap();
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
        .context("Failed setting S3 object retention")
        .unwrap();

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
        .context("Failed setting S3 legal hold")
        .unwrap();

    // Now `object_legal_hold` is correct.
    let legal_hold = s3_client
        .get_object_legal_hold()
        .bucket(&s3_store.bucket)
        .key(&key)
        .send()
        .await;
    assert!(legal_hold.is_ok(), "legal_hold: {legal_hold:#?}");
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
        prefix: None,
        force_path_style: None,
        object_lock,
        object_lock_legal_hold_status,
    }))
}

#[allow(clippy::borrowed_box)] // For convenience in call sites.
fn as_s3_store(object_store: &Box<dyn ObjectStore>) -> &'static S3Store {
    unsafe { &*(object_store.as_ref() as *const dyn ObjectStore as *const S3Store) }
}
