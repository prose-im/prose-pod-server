// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use anyhow::Context as _;
use bytes::Bytes;
use s3::{error::SdkError, presigning::PresigningConfig, types::CompletedPart};
use std::io::{self, Read, Write};

use crate::{config::StorageS3Config, util::saturating_i64_to_u64};

use super::prelude::*;

/// 8MiB.
const UPLOAD_PART_SIZE: usize = 8 * 1024 * 1024;

pub struct S3Store {
    pub client: s3::Client,
    pub bucket: String,
}

impl S3Store {
    pub fn from_config(config: &StorageS3Config) -> Self {
        use secrecy::ExposeSecret as _;

        let StorageS3Config {
            bucket_name,
            region,
            endpoint_url,
            access_key,
            secret_key,
            session_token,
            force_path_style,
        } = config;

        let creds = s3::config::Credentials::new(
            String::clone(access_key),
            secret_key.expose_secret(),
            session_token
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
            None,
            "config",
        );

        let s3_config = {
            let mut builder = s3::Config::builder()
                .region(s3::config::Region::new(String::clone(region)))
                .endpoint_url(endpoint_url)
                .credentials_provider(creds)
                .behavior_version_latest();

            if let Some(force_path_style) = force_path_style {
                builder = builder.force_path_style(*force_path_style);
            }

            builder.build()
        };

        let client = s3::Client::from_conf(s3_config);

        Self {
            client,
            bucket: String::clone(bucket_name),
        }
    }
}

#[async_trait::async_trait]
impl ObjectStore for S3Store {
    async fn writer(&self, key: &str) -> Result<Box<DynObjectWriter>, anyhow::Error> {
        let writer = S3Writer::new(self.client.clone(), &self.bucket, key).await?;
        Ok(Box::new(writer))
    }

    async fn reader(&self, key: &str) -> Result<Box<DynObjectReader>, ReadObjectError> {
        match self.exists_(key).await {
            Ok(_) => {
                let reader = S3Reader::new(self.client.clone(), &self.bucket, key);
                Ok(Box::new(reader))
            }
            Err(err) => Err(err),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, anyhow::Error> {
        match self.exists_(key).await {
            Ok(_) => Ok(true),
            Err(ReadObjectError::ObjectNotFound(_)) => Ok(false),
            Err(ReadObjectError::Other(e)) => Err(e),
        }
    }

    async fn find(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        let mut results: Vec<ObjectMetadata> = Vec::new();
        let mut continuation_token = None;

        loop {
            let resp = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix)
                .set_continuation_token(continuation_token.clone())
                .send()
                .await
                .context("Failed listing S3 objects")?;

            results.extend(resp.contents().into_iter().filter_map(|obj| {
                match (obj.key(), obj.size()) {
                    (Some(key), Some(size)) => Some(ObjectMetadata {
                        file_name: key.to_owned(),
                        size_bytes: saturating_i64_to_u64(size),
                    }),
                    _ => None,
                }
            }));

            if resp.is_truncated().unwrap_or(false) {
                continuation_token = resp.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        Ok(results)
    }

    async fn list_all_after(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        let mut results: Vec<ObjectMetadata> = Vec::new();
        let mut continuation_token = None;

        loop {
            let resp = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .start_after(prefix)
                .set_continuation_token(continuation_token.clone())
                .send()
                .await
                .context("Failed listing S3 objects")?;

            results.extend(resp.contents().into_iter().filter_map(|obj| {
                match (obj.key(), obj.size()) {
                    (Some(key), Some(size)) => Some(ObjectMetadata {
                        file_name: key.to_owned(),
                        size_bytes: saturating_i64_to_u64(size),
                    }),
                    _ => None,
                }
            }));

            if resp.is_truncated().unwrap_or(false) {
                continuation_token = resp.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        Ok(results)
    }

    async fn metadata(&self, key: &str) -> Result<ObjectMetadata, ReadObjectError> {
        let meta = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("Failed getting S3 object metadata")
            .map_err(ReadObjectError::ObjectNotFound)?;

        let size: u64 = saturating_i64_to_u64(
            meta.content_length()
                .expect("S3 object has no content_length"),
        );

        Ok(ObjectMetadata {
            file_name: key.to_owned(),
            size_bytes: size,
        })
    }

    async fn download_url(
        &self,
        key: &str,
        ttl: &std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        let presigned = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(PresigningConfig::expires_in(*ttl)?)
            .await?;

        Ok(presigned.uri().to_owned())
    }

    async fn delete(&self, key: &str) -> Result<(), anyhow::Error> {
        let _ = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;
        Ok(())
    }
}

impl S3Store {
    async fn exists_(
        &self,
        key: &str,
    ) -> Result<s3::operation::head_object::HeadObjectOutput, ReadObjectError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => Ok(output),
            Err(SdkError::ServiceError(e)) if e.err().is_not_found() => Err(
                ReadObjectError::ObjectNotFound(anyhow::Error::from(SdkError::ServiceError(e))),
            ),
            Err(err) => Err(ReadObjectError::ObjectNotFound(anyhow::Error::from(err))),
        }
    }
}

// MARK: Writer

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
            buf: Vec::with_capacity(UPLOAD_PART_SIZE),
            parts: Vec::new(),
            part_number: 1,
        })
    }

    async fn flush_part(&mut self) -> Result<(), anyhow::Error> {
        tracing::trace!(key = self.key, "S3Writer::flush_part");

        if self.buf.is_empty() {
            tracing::trace!(
                key = self.key,
                "S3 upload part flush skipped: Buffer is empty."
            );
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

        tracing::trace!(key = self.key, "S3 upload part flush completed.");

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
                s3::types::CompletedMultipartUpload::builder()
                    .set_parts(Some(self.parts))
                    .build(),
            )
            .send()
            .await
            .context("S3 multipart upload complete failed")?;

        tracing::trace!(key = self.key, "S3 multipart upload completed.");

        Ok(())
    }
}

impl Write for S3Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);

        if self.buf.len() >= UPLOAD_PART_SIZE {
            tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(self.flush_part())
            })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        } else {
            tracing::trace!(
                key = self.key,
                "S3 upload part flush skipped: Buffer not full ({size_before} + {written} = {size} < {UPLOAD_PART_SIZE}).",
                size_before = self.buf.len() - buf.len(),
                written = buf.len(),
                size = self.buf.len()
            );
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        tracing::trace!(key = self.key, "S3Writer::flush");
        Ok(())
    }
}

impl super::Finalizable for S3Writer {
    fn finalize(self: Box<Self>) -> Result<(), anyhow::Error> {
        tokio::task::block_in_place(move || {
            tokio::runtime::Handle::current().block_on(self.complete())
        })
    }
}

impl super::ObjectWriter for S3Writer {}

// MARK: Reader

pub struct S3Reader {
    client: s3::Client,
    bucket: String,
    key: String,
    stream: Option<s3::primitives::ByteStream>,
    buf: Bytes,
}

impl S3Reader {
    pub fn new(client: s3::Client, bucket: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
            key: key.into(),
            stream: None,
            buf: Bytes::new(),
        }
    }
}

impl S3Reader {
    fn ensure_stream(&mut self) -> Result<&mut s3::primitives::ByteStream, io::Error> {
        crate::util::get_or_try_insert(&mut self.stream, || {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    self.client
                        .get_object()
                        .bucket(&self.bucket)
                        .key(&self.key)
                        .send()
                        .await
                        .map(|resp| resp.body)
                        .context("Failed to open S3 object for reading")
                })
            })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
        })
    }
}

impl Read for S3Reader {
    fn read(&mut self, out: &mut [u8]) -> Result<usize, io::Error> {
        // Drain current buffer first.
        if !self.buf.is_empty() {
            let n = std::cmp::min(out.len(), self.buf.len());
            out[..n].copy_from_slice(&self.buf[..n]);
            self.buf = self.buf.slice(n..);
            return Ok(n);
        }

        let stream = self.ensure_stream()?;

        // Fetch next chunk from stream.
        let chunk = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(stream.next())
        });

        match chunk {
            None => Ok(0), // EOF
            Some(Err(e)) => Err(io::Error::new(io::ErrorKind::Other, e)),
            Some(Ok(chunk)) => {
                let n = std::cmp::min(out.len(), chunk.len());
                out[..n].copy_from_slice(&chunk[..n]);
                self.buf = chunk.slice(n..);
                Ok(n)
            }
        }
    }
}
