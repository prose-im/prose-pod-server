// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use prose_backup::stores::{Finalizable, ObjectWriter, prelude::*};

/// A store that only allows writing a certain amount of bytes. This is useful
/// to test failure cases.
#[derive(Debug)]
pub struct LimitedStore {
    inner: Box<dyn ObjectStore>,
    limit: u64,
    fail_finalize: bool,
}

impl LimitedStore {
    pub fn wrap(store: &mut Box<dyn ObjectStore>, limit: u64, fail_finalize: bool) {
        unsafe {
            // Move out without dropping.
            let inner = std::ptr::read(store);

            // Construct new value.
            let new = Box::new(Self {
                inner,
                limit,
                fail_finalize,
            });

            // Write the new value WITHOUT dropping the destination,
            // since we already moved out of it above.
            std::ptr::write(store, new);
        }
    }
}

#[allow(unused_variables)]
#[async_trait::async_trait]
impl ObjectStore for LimitedStore {
    async fn writer(&self, key: &str) -> Result<Box<DynObjectWriter>, anyhow::Error> {
        let inner = self.inner.writer(key).await?;
        Ok(Box::new(LimitedWriter {
            inner,
            progress: 0,
            limit: self.limit,
            fail_finalize: self.fail_finalize,
        }))
    }

    async fn reader(&self, key: &str) -> Result<Box<DynObjectReader>, ReadObjectError> {
        unimplemented!()
    }

    async fn exists(&self, key: &str) -> Result<bool, anyhow::Error> {
        unimplemented!()
    }

    async fn find(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        unimplemented!()
    }

    async fn list_all_after(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        unimplemented!()
    }

    async fn metadata(&self, key: &str) -> Result<ObjectMetadata, ReadObjectError> {
        unimplemented!()
    }

    async fn download_url(
        &self,
        key: &str,
        ttl: &std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        unimplemented!()
    }

    async fn delete(&self, key: &str) -> Result<DeletedState, anyhow::Error> {
        self.inner.delete(key).await
    }

    async fn delete_all(&self, prefix: &str) -> Result<BulkDeleteOutput, anyhow::Error> {
        self.inner.delete_all(prefix).await
    }
}

struct LimitedWriter {
    inner: Box<DynObjectWriter>,
    progress: u64,
    limit: u64,
    fail_finalize: bool,
}

impl std::io::Write for LimitedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.inner.write(buf) {
            ok @ Ok(len) => {
                // SAFETY: We’ll never hit `u64::MAX` in tests.
                self.progress += len as u64;

                if self.progress > self.limit {
                    Err(std::io::Error::other("LimitedWriter limit reached."))
                } else {
                    ok
                }
            }
            err => err,
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Finalizable for LimitedWriter {
    fn finalize(self: Box<Self>) -> Result<(), anyhow::Error> {
        if self.fail_finalize {
            Err(anyhow::Error::msg("LimitedWriter fail."))
        } else {
            self.inner.finalize()
        }
    }
}

impl ObjectWriter for LimitedWriter {}
