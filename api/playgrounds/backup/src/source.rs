// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Read;

pub trait BackupSource {
    type Reader: Read;

    fn reader(&self, file_name: &str) -> Result<Self::Reader, anyhow::Error>;
}

#[cfg(feature = "destination_s3")]
pub use self::s3::S3Source;
#[cfg(feature = "destination_s3")]
mod s3 {
    use super::BackupSource;

    pub struct S3Source {
        client: s3::Client,
        bucket: String,
    }

    impl BackupSource for S3Source {
        type Reader = S3Reader;

        fn reader(&self, key: &str) -> Result<Self::Reader, anyhow::Error> {
            Ok(S3Reader::new(self.client.clone(), &self.bucket, key))
        }
    }

    // MARK: Reader

    use bytes::Bytes;
    use std::io::{self, Read};

    /// 8MB.
    const CHUNK_SIZE: u64 = 8 * 1024 * 1024;

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

            let range = format!("bytes={}-{}", self.offset, self.offset + CHUNK_SIZE - 1);

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
}

#[cfg(feature = "destination_file")]
pub use self::file::FileSource;
#[cfg(feature = "destination_file")]
mod file {
    use std::fs::File;
    use std::path::{Path, PathBuf};

    use anyhow::Context as _;

    use super::BackupSource;

    /// Read backups stored on disk.
    #[derive(Default)]
    pub struct FileSource {
        directory: PathBuf,
    }

    impl FileSource {
        pub fn new(directory: impl AsRef<Path>) -> Self {
            Self {
                directory: directory.as_ref().to_path_buf(),
            }
        }
    }

    impl BackupSource for FileSource {
        type Reader = File;

        fn reader(&self, file_name: &str) -> Result<Self::Reader, anyhow::Error> {
            assert!(
                !file_name.starts_with("/"),
                "File name should not start with a `/`"
            );

            File::options()
                .read(true)
                .open(self.directory.join(file_name))
                .context("Could not open backup file")
        }
    }
}
