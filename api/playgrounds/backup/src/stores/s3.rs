// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use anyhow::{Context as _, anyhow};
use bytes::Bytes;
use s3::{
    error::SdkError,
    presigning::PresigningConfig,
    types::{CompletedPart, ObjectLockLegalHoldStatus},
};
use std::{
    io::{self, Read, Write},
    time::SystemTime,
};

use crate::{config::StorageS3Config, util::saturating_i64_to_u64};

use super::prelude::*;

/// 8MiB.
const UPLOAD_PART_SIZE: usize = 8 * 1024 * 1024;

#[cfg_attr(feature = "test", derive(Clone))]
pub struct S3Store {
    pub client: s3::Client,
    pub bucket: String,
    pub object_lock: Option<crate::config::S3ObjectLockConfig>,
    pub object_lock_legal_hold_status: Option<s3::types::ObjectLockLegalHoldStatus>,
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
            object_lock,
            object_lock_legal_hold_status,
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
            object_lock: object_lock.clone(),
            object_lock_legal_hold_status: object_lock_legal_hold_status.clone(),
        }
    }
}

#[async_trait::async_trait]
impl ObjectStore for S3Store {
    async fn writer(&self, key: &str) -> Result<Box<DynObjectWriter>, anyhow::Error> {
        let writer = S3Writer::new(
            self.client.clone(),
            &self.bucket,
            key,
            self.object_lock.as_ref(),
            self.object_lock_legal_hold_status.as_ref(),
        )
        .await?;
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

    async fn delete(&self, key: &str) -> Result<DeletedState, anyhow::Error> {
        let output = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("Failed deleting S3 object")?;

        // FIXME: Doesn’t seem to work with Ceph (hence, with Hetzner).
        //   It’s annoying to make a separate request just to check that,
        //   so we’ll pretend the object was deleted. If one is using
        //   Object Locking or versioning, they are aware of this and
        //   likely have some cleanup configuration in place.
        if output.delete_marker() == Some(true) {
            Ok(DeletedState::MarkedForDeletion)
        } else {
            Ok(DeletedState::Deleted)
        }
    }

    async fn delete_all(&self, prefix: &str) -> Result<BulkDeleteOutput, anyhow::Error> {
        use s3::types::{Delete, ObjectIdentifier};

        let mut output = BulkDeleteOutput::default();

        // Find all objects matching prefix.
        // NOTE: If object versioning is enabled —which is always the case when
        //   object lock is enabled—, deleting an object without specifying an
        //   object ID only results in the creation of a delete marker
        //   (a.k.a. “tombstone”). To really delete an object, one must delete
        //   all of its existing versions.
        let objects = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .send()
            .await?;

        let identifiers: Vec<ObjectIdentifier> = objects
            .contents()
            .into_iter()
            .filter_map(|object| match object.key() {
                Some(object_key) => Some(
                    ObjectIdentifier::builder()
                        .key(object_key)
                        .build()
                        // SAFETY: `key` is set.
                        .unwrap(),
                ),
                None => None,
            })
            .collect();

        // Abort if no match found.
        if identifiers.is_empty() {
            tracing::debug!("Objects prefixed with `{prefix}` cannot be deleted: No match found.");
            return Ok(output);
        }

        // Bulk delete all objects.
        let results = self
            .client
            .delete_objects()
            .bucket(&self.bucket)
            .delete(
                Delete::builder()
                    .set_objects(Some(identifiers))
                    .build()
                    // SAFETY: `objects` is set.
                    .unwrap(),
            )
            .send()
            .await?;

        for object in results.deleted.unwrap_or_default() {
            let key = object.key().map_or(String::new(), str::to_owned);

            // FIXME: Doesn’t seem to work with Ceph (hence, with Hetzner).
            //   It’s annoying to make a separate request just to check that,
            //   so we’ll pretend the object was deleted. If one is using
            //   Object Locking or versioning, they are aware of this and
            //   likely have some cleanup configuration in place.
            if object.delete_marker() == Some(true) {
                output.marked_for_deletion.push(key);
            } else {
                output.deleted.push(key);
            }
        }

        for error in results.errors.unwrap_or_default() {
            let key = error.key().unwrap_or_default();
            let error = anyhow!("{error:?}").context(format!("Object `{key}` not deleted"));
            output.errors.push(error);
        }

        Ok(output)
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
    put_object_retention:
        Option<s3::operation::put_object_retention::builders::PutObjectRetentionFluentBuilder>,
    put_object_legal_hold:
        Option<s3::operation::put_object_legal_hold::builders::PutObjectLegalHoldFluentBuilder>,
}

impl S3Writer {
    pub async fn new(
        client: s3::Client,
        bucket: impl Into<String>,
        key: impl Into<String>,
        object_lock: Option<&crate::config::S3ObjectLockConfig>,
        object_lock_legal_hold_status: Option<&ObjectLockLegalHoldStatus>,
    ) -> Result<Self, anyhow::Error> {
        let bucket = bucket.into();
        let key = key.into();

        let response = client
            .create_multipart_upload()
            .bucket(&bucket)
            .key(&key)
            .send()
            .await
            .context("Failed creating S3 multipart upload")?;

        let put_object_retention = object_lock.map(|object_lock| {
            client
                .put_object_retention()
                .bucket(&bucket)
                .key(&key)
                .retention(
                    s3::types::ObjectLockRetention::builder()
                        .mode(object_lock.mode.clone())
                        .retain_until_date((SystemTime::now() + object_lock.duration).into())
                        .build(),
                )
        });
        let put_object_legal_hold =
            object_lock_legal_hold_status.map(|object_lock_legal_hold_status| {
                client
                    .put_object_legal_hold()
                    .bucket(&bucket)
                    .key(&key)
                    .legal_hold(
                        s3::types::ObjectLockLegalHold::builder()
                            .status(object_lock_legal_hold_status.to_owned())
                            .build(),
                    )
            });

        Ok(Self {
            client,
            bucket,
            key,
            upload_id: response.upload_id().unwrap().to_string(),
            buf: Vec::with_capacity(UPLOAD_PART_SIZE),
            parts: Vec::new(),
            part_number: 1,
            put_object_retention,
            put_object_legal_hold,
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

        // NOTE: With Hetzner, which uses Ceph, manual testing showed setting
        //   object retention or legal hold during a multipart upload silently
        //   gets ignored. To work around it, we set it in separate requests.
        //   It’s unfortunate we have to make three requests instead of one.
        if let Some(put_object_retention) = self.put_object_retention {
            put_object_retention
                .send()
                .await
                .context("Failed adding S3 object retention metadata")?;
        }
        if let Some(put_object_legal_hold) = self.put_object_legal_hold {
            put_object_legal_hold
                .send()
                .await
                .context("Failed adding S3 object legal hold metadata")?;
        }

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
