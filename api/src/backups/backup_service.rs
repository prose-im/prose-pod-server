// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod prelude {
    pub use std::sync::Arc;

    pub use crate::auth::CallerInfo;

    pub use super::{BackupId, BackupMetadata, BackupService, BackupServiceImpl};
}

use serde::Serialize;
use time::UtcDateTime;

use crate::{
    app_config::{BackupBackend, BackupsConfig},
    backups::BackupRepository,
    responders::Error,
};

use self::prelude::*;

#[derive(Debug, Clone)]
pub struct BackupService {
    pub implem: Arc<dyn BackupServiceImpl>,
}

impl BackupService {
    pub fn from_config(backups_config: &BackupsConfig) -> Result<Self, Error> {
        let repository = BackupRepository::from_config(backups_config)?;

        Ok(Self {
            implem: Arc::new(LiveBackupService {
                repository,
                zstd_compression_level: backups_config.zstd.compression_level,
            }),
        })
    }
}

#[async_trait::async_trait]
pub trait BackupServiceImpl: std::fmt::Debug + Sync + Send {
    async fn create_backup(&self, caller: &CallerInfo) -> Result<BackupMetadata, Error>;

    async fn get_backup(
        &self,
        backup_id: BackupId,
        caller: &CallerInfo,
    ) -> Result<Option<BackupMetadata>, Error>;

    async fn list_backups(&self, caller: &CallerInfo) -> Result<Vec<BackupMetadata>, Error>;

    async fn delete_backup(&self, backup_id: BackupId, caller: &CallerInfo) -> Result<(), Error>;
}

pub type BackupId = String;

#[derive(Debug, Clone)]
#[derive(Serialize)]
pub struct BackupMetadata {
    pub backup_id: BackupId,
    pub created_at: Option<UtcDateTime>,
    pub size_bytes: Option<u64>,
    pub checksum: Option<String>,
}

use self::live::*;
mod live {
    use std::fs::File;

    use anyhow::Context;
    use async_trait::async_trait;
    use aws_sdk_s3::error::SdkError;
    use time::{UtcDateTime, format_description::well_known::Rfc3339};
    use tokio::task;
    use tokio_util::bytes::Bytes;

    use crate::{
        backups::BackupRepository,
        errors,
        util::{NoPublicContext as _, PublicContext as _},
    };

    use super::*;

    #[derive(Debug)]
    pub struct LiveBackupService {
        pub repository: BackupRepository,
        pub zstd_compression_level: i32,
    }

    #[async_trait]
    impl BackupServiceImpl for LiveBackupService {
        #[tracing::instrument(level = "trace", skip_all)]
        async fn create_backup(&self, caller: &CallerInfo) -> Result<BackupMetadata, Error> {
            if !caller.is_admin() {
                return Err(errors::forbidden(format!(
                    "{} is not an admin.",
                    caller.jid
                )));
            }

            // Create fixed-size buffers to hold the streamed data.
            let mut backup_buf = [0u8; 4096];
            let mut integrity_check_buf = [0u8; 4096];

            let backup_stream = todo!();
            let integrity_check_stream = todo!();

            let backup_upload_stream = todo!();

            struct ArchivingConfig {
                paths: Vec<(&'static str, &'static str)>,
            }
            let archiving_config = ArchivingConfig {
                paths: vec![
                    ("/var/lib/prosody", "prosody-data"),
                    ("/etc/prosody", "prosody-config"),
                ],
            };
            fn archive<W: Write>(writer: W, config: &ArchivingConfig) -> io::Result<()> {
                let mut tar = tar::Builder::new(writer);

                // Add files to the tar stream
                tar.append_path("src/main.rs")?;
                tar.append_path("Cargo.toml")?;

                tar
            }

            let backup_writer = TeeWriter::new(backup_buf, integrity_check_buf);
            let stream = archive(compress(backup_writer), &archiving_config);

            let (a, b) = tokio::try_join!(s3_upload(backup_buf), s3_upload(integrity_check_buf))?;

            // ---

            // Generate a new random name for the backup.
            let now = UtcDateTime::now()
                .replace_millisecond(0)
                .expect("0 should be a valid millisecond")
                .format(&Rfc3339)
                .context("Could not get current time as RFC 3339")
                .no_public_context()?;
            let archive_name = format!("prose_{now}.tar.zst");

            // Create the backup file.
            // NOTE: Backups are heavy therefore we shouldn’t do it in memory.
            let archive_file = File::create(&archive_name)
                .context("Could not create backup archive file")
                .no_public_context()?;

            // Wrap the output writer in a streaming Zstd encoder
            let encoder = zstd::Encoder::new(archive_file, self.zstd_compression_level)
                .context("Could not create zstd encoder")
                .no_public_context()?;
            let mut encoder = encoder.auto_finish(); // ensures drop = finish

            // TAR builder writes into the encoder, which compresses as it streams
            let mut tar = tar::Builder::new(&mut encoder);

            // Add files to the tar stream
            tar.append_path("src/main.rs")?;
            tar.append_path("Cargo.toml")?;

            tar.finish()?; // finish tar stream

            // sdfsdf

            let archive = tar::Builder::new(archive_file);

            let archive_name = format!("{archive_name}.zst");

            todo!();
            let data = Bytes::new();

            let backup_id = archive_name;

            let metadata = self
                .repository
                .create_backup(&backup_id, data)
                .await
                .public_context("BACKUP_FAILED", "Backup failed.")?;

            Ok(metadata)
        }

        #[tracing::instrument(level = "trace", skip_all)]
        async fn get_backup(
            &self,
            backup_id: BackupId,
            caller: &CallerInfo,
        ) -> Result<Option<BackupMetadata>, Error> {
            if !caller.is_admin() {
                return Err(errors::forbidden(format!(
                    "{} is not an admin.",
                    caller.jid
                )));
            }

            let key = Self::object_key(&backup_id);

            let result = self
                .client
                .head_object()
                .bucket(&self.bucket)
                .key(&key)
                .send()
                .await;

            match result {
                Ok(head) => {
                    let created_at = head
                        .last_modified()
                        .map(|lm| OffsetDateTime::from_unix_timestamp(lm.secs()))
                        .transpose()
                        .context("invalid timestamp")?
                        .unwrap_or_else(OffsetDateTime::now_utc);

                    todo!()
                    // Ok(Some(BackupMetadata {
                    //     backup_id,
                    //     size_bytes: head.content_length() as u64,
                    //     created_at,
                    //     checksum: head.e_tag().unwrap_or("").trim_matches('"').to_string(),
                    // }))
                }
                Err(err) => {
                    // If it is 404
                    if matches!(err, SdkError::ServiceError(error) if error.err().is_not_found()) {
                        return Ok(None);
                    }
                    Err(err).context("failed to head backup object")
                }
            }
        }

        #[tracing::instrument(level = "trace", skip_all)]
        async fn list_backups(&self, caller: &CallerInfo) -> Result<Vec<BackupMetadata>, Error> {
            if !caller.is_admin() {
                return Err(errors::forbidden(format!(
                    "{} is not an admin.",
                    caller.jid
                )));
            }

            todo!()
        }

        #[tracing::instrument(level = "trace", skip_all, fields(backup_id))]
        async fn delete_backup(
            &self,
            backup_id: BackupId,
            caller: &CallerInfo,
        ) -> Result<(), Error> {
            if !caller.is_admin() {
                return Err(errors::forbidden(format!(
                    "{} is not an admin.",
                    caller.jid
                )));
            }

            self.repository.delete_backup(&backup_id).await
        }
    }

    use std::io::{self, Write};

    pub struct TeeWriter<W1, W2> {
        a: W1,
        b: W2,
    }

    impl<W1, W2> TeeWriter<W1, W2> {
        pub fn new(a: W1, b: W2) -> Self {
            Self { a, b }
        }
    }

    impl<W1: Write, W2: Write> Write for TeeWriter<W1, W2> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            // Write to first writer
            let n = self.a.write(buf)?;

            // Write the same amount to the second writer
            // If this fails, return that error
            self.b.write_all(&buf[..n])?;

            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.a.flush()?;
            self.b.flush()?;
            Ok(())
        }
    }
}

// MARK: - Boilerplate

impl std::ops::Deref for BackupService {
    type Target = Arc<dyn BackupServiceImpl>;

    fn deref(&self) -> &Self::Target {
        &self.implem
    }
}
