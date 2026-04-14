// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use prose_backup::stores::prelude::*;

/// Store which doesn’t store data. Just dismisses it.
#[derive(Debug)]
pub struct SinkStore;

#[async_trait::async_trait]
impl ObjectStore for SinkStore {
    async fn writer(&self, _file_name: &str) -> Result<Box<DynObjectWriter>, anyhow::Error> {
        Ok(Box::new(Sink::new()))
    }

    async fn reader(&self, _file_name: &str) -> Result<Box<DynObjectReader>, ReadObjectError> {
        unimplemented!()
    }

    async fn exists(&self, _key: &str) -> Result<bool, anyhow::Error> {
        unimplemented!()
    }

    async fn find(&self, _prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        unimplemented!()
    }

    async fn list_all_after(&self, _prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        unimplemented!()
    }

    async fn metadata(&self, _file_name: &str) -> Result<ObjectMetadata, ReadObjectError> {
        unimplemented!()
    }

    async fn download_url(
        &self,
        _file_name: &str,
        _ttl: &std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        unimplemented!()
    }

    async fn delete(&self, _file_name: &str) -> Result<DeletedState, anyhow::Error> {
        Ok(DeletedState::Deleted)
    }

    async fn delete_all(&self, _prefix: &str) -> Result<BulkDeleteOutput, anyhow::Error> {
        Ok(BulkDeleteOutput::default())
    }
}

#[repr(transparent)]
struct Sink(std::io::Sink);

impl Sink {
    fn new() -> Self {
        Self(std::io::sink())
    }
}

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl prose_backup::stores::Finalizable for Sink {
    fn finalize(self: Box<Self>) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

impl prose_backup::stores::ObjectWriter for Sink {}
