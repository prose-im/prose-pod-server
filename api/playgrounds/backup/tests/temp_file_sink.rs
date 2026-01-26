// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    fs::File,
    path::{Path, PathBuf},
};

use prose_backup::sink::{BackupSink, FileSink};

pub struct TempFileSink {
    prefix: PathBuf,
    inner: FileSink,
}

impl TempFileSink {
    pub fn new(prefix: impl AsRef<Path>) -> Self {
        Self {
            prefix: prefix.as_ref().to_path_buf(),
            inner: FileSink::new(true, 0o600),
        }
    }
}

impl BackupSink for TempFileSink {
    type BackupWriter = File;
    type IntegrityCheckWriter = File;

    fn writer(&self, file_name: &str) -> Result<Self::BackupWriter, anyhow::Error> {
        self.inner
            .writer(self.prefix.join(file_name).to_str().unwrap())
    }

    fn integrity_check_writer(
        &self,
        file_name: &str,
    ) -> Result<Self::IntegrityCheckWriter, anyhow::Error> {
        self.inner
            .integrity_check_writer(self.prefix.join(file_name).to_str().unwrap())
    }
}
