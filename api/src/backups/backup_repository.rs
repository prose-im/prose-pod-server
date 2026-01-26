// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod prelude {
    pub use async_trait::async_trait;
    pub use tokio_util::bytes::Bytes;

    pub use crate::{
        backups::{
            backup_repository::BackupRepositoryImpl,
            backup_service::{BackupId, BackupMetadata},
        },
        models::AuthToken,
        responders::Error,
        util::either::Either,
    };
}

use std::sync::Arc;

use crate::{
    app_config::{BackupBackend, BackupsConfig},
    errors,
};

use self::prelude::*;

#[derive(Debug, Clone)]
pub struct BackupRepository {
    pub implem: Arc<dyn BackupRepositoryImpl>,
}

impl BackupRepository {
    pub fn from_config(backups_config: &BackupsConfig) -> Result<Self, Error> {
        match backups_config.backend {
            BackupBackend::S3 => {
                let Some(ref s3_config) = backups_config.s3 else {
                    return Err(errors::missing_configuration("backups.s3"));
                };

                let repository = S3BackupRepository::new(s3_config);

                Ok(Self {
                    implem: Arc::new(repository),
                })
            }
        }
    }
}

#[async_trait]
pub trait BackupRepositoryImpl: std::fmt::Debug + Sync + Send {
    async fn list_backups(&self) -> Result<Vec<BackupMetadata>, anyhow::Error>;

    async fn get_backup(
        &self,
        backup_id: &BackupId,
    ) -> Result<Option<BackupMetadata>, anyhow::Error>;

    async fn create_backup(
        &self,
        backup_id: &BackupId,
        backup_data: Bytes,
    ) -> Result<BackupMetadata, anyhow::Error>;

    async fn delete_backup(&self, backup_id: &BackupId) -> Result<(), anyhow::Error>;
}

#[derive(Debug)]
pub struct UsersStats {
    pub count: usize,
}

pub use self::s3::*;
mod s3 {
    use anyhow::{Context as _, anyhow};
    use aws_sdk_s3::{
        Client, Config,
        config::{Credentials, Region},
        error::SdkError,
        operation::{get_object::GetObjectOutput, put_object::PutObjectOutput},
        primitives::ByteStream,
        types::{Delete, Object, ObjectIdentifier},
    };
    use time::{OffsetDateTime, UtcDateTime};

    use crate::app_config::S3Config;

    use super::*;

    #[derive(Debug)]
    pub struct S3BackupRepository {
        pub client: Client,
        pub bucket: String,
    }

    impl S3BackupRepository {
        pub fn new(config: &S3Config) -> Self {
            // 1. Provide your own AWS credentials
            let credentials = Credentials::from_keys(
                "YOUR_ACCESS_KEY_ID",
                "YOUR_SECRET_ACCESS_KEY",
                None, // optional session token
            );

            // 2. Provide your region
            let region = Region::new("us-east-1");

            // 3. Build the AWS SDK config manually
            let s3_config = Config::builder()
                .credentials_provider(credentials)
                .region(region)
                .build();

            // 4. Create the S3 client
            let client = Client::from_conf(s3_config);

            Self {
                client,
                bucket: config.bucket,
            }
        }
    }

    #[async_trait]
    impl BackupRepositoryImpl for S3BackupRepository {
        async fn list_backups(&self) -> Result<Vec<BackupMetadata>, anyhow::Error> {
            let response = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .send()
                .await
                .context("Failed to list backups")?;
            let objects = response.contents();

            // Pre-allocate `objects.len() / 3` as backups can have a checksum
            // and a signature. If there is no signature, then we allocated too
            // few elements but that’s negligible.
            let mut results = Vec::with_capacity(objects.len() / 3);

            let objects = objects.into_iter().filter(|obj| match obj.key() {
                None => false,
                Some(key) => !key.ends_with(".sig") && !key.ends_with(".sha256"),
            });

            for object in objects {
                let Some(key) = object.key() else { continue };
                let backup_id = key.to_owned();

                let created_at_opt = object
                    .creation_date()
                    .context(format!("Invalid creation date for '{backup_id}'"))
                    .inspect_err(|err| tracing::warn!("{err:?}"))
                    .ok();

                let checksum_opt = self
                    .get_backup_checksum(&backup_id)
                    .await
                    .inspect_err(|err| tracing::warn!("{err:?}"))
                    .ok();

                let size_bytes = object.size().map_or_else(
                    || {
                        tracing::warn!("Backup '{backup_id}' has no size.");
                        None
                    },
                    |size: i64| Some(size as u64),
                );

                results.push(BackupMetadata {
                    backup_id,
                    size_bytes,
                    checksum: checksum_opt,
                    created_at: created_at_opt,
                });
            }

            Ok(results)
        }

        async fn get_backup(
            &self,
            backup_id: &BackupId,
        ) -> Result<Option<BackupMetadata>, anyhow::Error> {
            let response = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(backup_id)
                .send()
                .await;
            let object = match response {
                Ok(object) => object,
                Err(SdkError::ServiceError(error)) if error.err().is_no_such_key() => {
                    return Ok(None);
                }
                // TODO: Handle `InvalidObjectState`.
                Err(err) => return Err(anyhow!(err).context("Failed to get backup")),
            };

            let backup_id = backup_id.to_owned();

            let created_at_opt = object
                .creation_date()
                .context(format!("Invalid creation date for '{backup_id}'"))
                .inspect_err(|err| tracing::warn!("{err:?}"))
                .ok();

            let checksum_opt = self
                .get_backup_checksum(&backup_id)
                .await
                .inspect_err(|err| tracing::warn!("{err:?}"))
                .ok();

            let size_bytes = object.content_length().map_or_else(
                || {
                    tracing::warn!("Backup '{backup_id}' has no size.");
                    None
                },
                |size: i64| Some(size as u64),
            );

            Ok(Some(BackupMetadata {
                backup_id,
                size_bytes,
                checksum: checksum_opt,
                created_at: created_at_opt,
            }))
        }

        async fn create_backup(
            &self,
            backup_id: &BackupId,
            backup_data: Bytes,
        ) -> Result<BackupMetadata, anyhow::Error> {
            let response = self
                .client
                .put_object()
                .bucket(&self.bucket)
                .key(backup_id)
                .if_none_match("*")
                .body(ByteStream::from(backup_data))
                .send()
                .await;
            match response {
                Ok(_object) => {}
                Err(SdkError::ServiceError(error)) if error.raw().status().as_u16() == 412 => {
                    return Err(anyhow!("Backup name conflict"));
                }
                Err(err) => return Err(anyhow!(err).context("Failed to upload backup")),
            };

            let backup_id = backup_id.to_owned();

            // NOTE: We cannot read all metadata from the given response as it
            //   does not contain “last modified” nor creation date metadata.
            let metadata = self
                .get_backup_metadata(&backup_id)
                .await?
                .context("Could not get backup after uploading")?;

            Ok(metadata)
        }

        async fn delete_backup(&self, backup_id: &BackupId) -> Result<(), anyhow::Error> {
            let response = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(backup_id)
                .send()
                .await
                .context("Could not list backup objects")?;

            if response.contents().is_empty() {
                return Ok(());
            }

            let identifiers: Vec<ObjectIdentifier> = response
                .contents()
                .into_iter()
                .flat_map(|obj| obj.key())
                .flat_map(|key| ObjectIdentifier::builder().key(key).build())
                .collect();

            let delete = Delete::builder()
                .set_objects(Some(identifiers))
                .build()
                .context("Could not build S3 delete request")?;

            let response = self
                .client
                .delete_objects()
                .bucket(&self.bucket)
                .delete(delete)
                .send()
                .await
                .context("Failed to delete backup")?;

            if response.errors().is_empty() {
                let keys = response
                    .deleted()
                    .into_iter()
                    .map(|obj| obj.key().unwrap_or_default())
                    .collect::<Vec<_>>();
                tracing::info!("Backup objects deleted: {keys:?}");
                Ok(())
            } else {
                Err(anyhow!(
                    "Error deleting backup '{backup_id}': {:?}",
                    response.errors()
                ))
            }
        }
    }

    impl S3BackupRepository {
        async fn get_backup_checksum(&self, backup_id: &BackupId) -> Result<String, anyhow::Error> {
            let checksum_data = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(format!("{backup_id}.sha256"))
                .send()
                .await
                .context(format!("Could not get checksum for backup '{backup_id}'"))?;

            let checksum_bytes = checksum_data
                .body
                .collect()
                .await
                .context(format!("Could not read checksum for backup '{backup_id}'"))?;

            let checksum = String::from_utf8(checksum_bytes.into_bytes().to_vec())
                .context(format!("Invalid checksum for backup '{backup_id}'"))?;

            Ok(checksum)
        }

        async fn get_backup_metadata(
            &self,
            backup_id: &BackupId,
        ) -> Result<Option<BackupMetadata>, anyhow::Error> {
            let response = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(backup_id)
                .send()
                .await;
            let object = match response {
                Ok(object) => object,
                Err(SdkError::ServiceError(error)) if error.err().is_no_such_key() => {
                    return Ok(None);
                }
                // TODO: Handle `InvalidObjectState`.
                Err(err) => return Err(anyhow!(err).context("Failed to get backup")),
            };

            let backup_id = backup_id.to_owned();

            let created_at_opt = object
                .creation_date()
                .context(format!("Invalid creation date for '{backup_id}'"))
                .inspect_err(|err| tracing::warn!("{err:?}"))
                .ok();

            let checksum_opt = self
                .get_backup_checksum(&backup_id)
                .await
                .inspect_err(|err| tracing::warn!("{err:?}"))
                .ok();

            let size_bytes = object.content_length().map_or_else(
                || {
                    tracing::warn!("Backup '{backup_id}' has no size.");
                    None
                },
                |size: i64| Some(size as u64),
            );

            Ok(Some(BackupMetadata {
                backup_id,
                size_bytes,
                checksum: checksum_opt,
                created_at: created_at_opt,
            }))
        }
    }

    trait ObjectExt {
        fn creation_date(&self) -> Result<UtcDateTime, anyhow::Error>;
    }

    impl ObjectExt for Object {
        fn creation_date(&self) -> Result<UtcDateTime, anyhow::Error> {
            match self.last_modified() {
                Some(date) => UtcDateTime::from_unix_timestamp_nanos(date.as_nanos())
                    .context("Invalid “last modified” date"),
                None => Err(anyhow!("No “last modified” date.")),
            }
        }
    }

    impl ObjectExt for GetObjectOutput {
        fn creation_date(&self) -> Result<UtcDateTime, anyhow::Error> {
            match self.last_modified() {
                Some(date) => UtcDateTime::from_unix_timestamp_nanos(date.as_nanos())
                    .context("Invalid “last modified” date"),
                None => Err(anyhow!("No “last modified” date.")),
            }
        }
    }
}

// MARK: - Boilerplate

impl std::ops::Deref for BackupRepository {
    type Target = Arc<dyn BackupRepositoryImpl>;

    fn deref(&self) -> &Self::Target {
        &self.implem
    }
}
