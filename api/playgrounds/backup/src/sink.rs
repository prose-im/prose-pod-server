// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Write;

/// Where to save backups.
/// By default, backups are uploaded to a S3-compliant object storage bucket.
pub trait BackupSink {
    type Writer: Write + Send + Sync;

    fn writer(&self, file_name: &str) -> Result<Self::Writer, anyhow::Error>;
}

#[cfg(feature = "destination_s3")]
pub use self::s3::S3Sink;
#[cfg(feature = "destination_s3")]
mod s3 {
    use anyhow::Context as _;
    use bytes::Bytes;
    use s3::types::CompletedPart;
    use std::io::{self, Write};

    use super::BackupSink;

    pub struct S3Sink {
        client: s3::Client,
        bucket: String,
    }

    impl BackupSink for S3Sink {
        type Writer = S3Writer;

        fn writer(&self, file_name: &str) -> Result<Self::Writer, anyhow::Error> {
            tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    S3Writer::new(self.client.clone(), &self.bucket, file_name).await
                })
            })
        }
    }

    // MARK: Writer

    /// 8MB.
    const PART_SIZE: usize = 8 * 1024 * 1024;

    pub struct S3Writer {
        client: s3::Client,
        bucket: String,
        key: String,
        upload_id: String,
        buf: Vec<u8>,
        parts: Vec<CompletedPart>,
        part_number: i32,
    }

    impl S3Writer {
        pub async fn new(
            client: s3::Client,
            bucket: impl Into<String>,
            key: impl Into<String>,
        ) -> Result<Self, anyhow::Error> {
            let bucket = bucket.into();
            let key = key.into();

            let resp = client
                .create_multipart_upload()
                .bucket(&bucket)
                .key(&key)
                .send()
                .await
                .context("Failed creating S3 multipart upload")?;

            Ok(Self {
                client,
                bucket,
                key,
                upload_id: resp.upload_id().unwrap().to_string(),
                buf: Vec::with_capacity(PART_SIZE),
                parts: Vec::new(),
                part_number: 1,
            })
        }

        async fn flush_part(&mut self) -> Result<(), anyhow::Error> {
            if self.buf.is_empty() {
                return Ok(());
            }

            let body = Bytes::copy_from_slice(&self.buf);

            let resp = self
                .client
                .upload_part()
                .bucket(&self.bucket)
                .key(&self.key)
                .upload_id(&self.upload_id)
                .part_number(self.part_number)
                .body(body.into())
                .send()
                .await
                .context("S3 multipart upload flush failed")?;

            self.parts.push(
                CompletedPart::builder()
                    .part_number(self.part_number)
                    .e_tag(resp.e_tag().unwrap())
                    .build(),
            );

            self.part_number += 1;
            self.buf.clear();
            Ok(())
        }

        pub async fn complete(mut self) -> Result<(), anyhow::Error> {
            self.flush_part().await?;

            self.client
                .complete_multipart_upload()
                .bucket(&self.bucket)
                .key(&self.key)
                .upload_id(&self.upload_id)
                .multipart_upload(
                    aws_sdk_s3::types::CompletedMultipartUpload::builder()
                        .set_parts(Some(self.parts))
                        .build(),
                )
                .send()
                .await
                .context("S3 multipart upload complete failed")?;

            Ok(())
        }
    }

    impl Write for S3Writer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buf.extend_from_slice(buf);

            if self.buf.len() >= PART_SIZE {
                // blocking shim; caller expected to be in async context
                let rt = tokio::runtime::Handle::current();
                rt.block_on(self.flush_part())
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            }

            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(feature = "destination_file")]
pub use self::file::FileSink;
#[cfg(feature = "destination_file")]
mod file {
    use std::{
        fs::File,
        os::unix::fs::OpenOptionsExt as _,
        path::{Path, PathBuf},
    };

    use anyhow::Context as _;

    use super::BackupSink;

    pub struct FileSink {
        directory: PathBuf,
        overwrite: bool,
        mode: u32,
    }

    impl FileSink {
        pub fn overwrite(mut self, overwrite: bool) -> Self {
            self.overwrite = overwrite;
            self
        }

        pub fn mode(mut self, mode: u32) -> Self {
            self.mode = mode;
            self
        }

        pub fn directory(mut self, directory: impl AsRef<Path>) -> Self {
            self.directory = directory.as_ref().to_path_buf();
            self
        }
    }

    impl Default for FileSink {
        fn default() -> Self {
            Self {
                directory: PathBuf::new(),
                overwrite: false,
                mode: 0o600,
            }
        }
    }

    impl BackupSink for FileSink {
        type Writer = File;

        fn writer(&self, file_name: &str) -> Result<Self::Writer, anyhow::Error> {
            assert!(
                !file_name.starts_with("/"),
                "File name should not start with a `/`"
            );

            File::options()
                .create(true)
                .create_new(!self.overwrite)
                .write(true)
                .truncate(self.overwrite)
                .mode(self.mode)
                .open(self.directory.join(file_name))
                .context("Failed opening file")
        }
    }
}
