// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use anyhow::Context as _;
use bytes::Bytes;
use s3::types::CompletedPart;
use std::io::{self, Read, Write};
use time::UtcDateTime;

use crate::util::saturating_i64_to_u64;

use super::{ObjectMetadata, ObjectStore};

/// 8MB.
const UPLOAD_PART_SIZE: usize = 8 * 1024 * 1024;

/// 8MB.
const READ_CHUNK_SIZE: u64 = 8 * 1024 * 1024;

pub struct S3Store {
    pub client: s3::Client,
    pub bucket: String,
}

impl ObjectStore for S3Store {
    type Writer = S3Writer;
    type Reader = S3Reader;

    async fn writer(&self, key: &str) -> Result<S3Writer, anyhow::Error> {
        tokio::task::block_in_place(move || {
            tokio::runtime::Handle::current().block_on(async move {
                S3Writer::new(self.client.clone(), &self.bucket, key).await
            })
        })
    }

    async fn reader(&self, key: &str) -> Result<Self::Reader, anyhow::Error> {
        Ok(S3Reader::new(self.client.clone(), &self.bucket, key))
    }

    async fn list_all(&self) -> Result<Vec<String>, anyhow::Error> {
        let mut keys = Vec::new();
        let mut continuation_token = None;

        loop {
            let resp = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .set_continuation_token(continuation_token.clone())
                .send()
                .await
                .context("Failed listing S3 objects")?;

            keys.extend(
                resp.contents()
                    .into_iter()
                    .filter_map(|obj| obj.key().map(ToOwned::to_owned)),
            );

            if resp.is_truncated().unwrap_or(false) {
                continuation_token = resp.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        Ok(keys)
    }

    async fn metadata(&self, key: &str) -> Result<ObjectMetadata, anyhow::Error> {
        let meta = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("Failed getting S3 object metadata")?;

        let created_at: &s3::primitives::DateTime = meta
            .last_modified()
            .context("S3 object has no last_modified timestamp")?;

        let creation_date: UtcDateTime = UtcDateTime::from_unix_timestamp(created_at.secs())
            .context("Failed converting S3 timestamp to OffsetDateTime")?;

        let size: u64 = saturating_i64_to_u64(
            meta.content_length()
                .expect("S3 object has no content_length"),
        );

        Ok(ObjectMetadata {
            file_name: key.to_owned(),
            creation_date: creation_date,
            size,
        })
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

        if self.buf.len() >= UPLOAD_PART_SIZE {
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

// MARK: Reader

pub struct S3Reader {
    client: s3::Client,
    bucket: String,
    key: String,
    buf: Bytes,
    pos: usize,
    offset: u64,
    eof: bool,
}

impl S3Reader {
    pub fn new(client: s3::Client, bucket: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
            key: key.into(),
            buf: Bytes::new(),
            pos: 0,
            offset: 0,
            eof: false,
        }
    }

    async fn refill(&mut self) -> anyhow::Result<()> {
        if self.eof {
            return Ok(());
        }

        let range = format!(
            "bytes={}-{}",
            self.offset,
            self.offset + READ_CHUNK_SIZE - 1
        );

        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .range(range)
            .send()
            .await?;

        let data = resp.body.collect().await?.into_bytes();

        if data.is_empty() {
            self.eof = true;
        } else {
            self.offset += data.len() as u64;
            self.buf = data;
            self.pos = 0;
        }

        Ok(())
    }
}

impl Read for S3Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.buf.len() && !self.eof {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(self.refill())
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        }

        if self.pos == self.buf.len() {
            return Ok(0);
        }

        let n = std::cmp::min(out.len(), self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}
