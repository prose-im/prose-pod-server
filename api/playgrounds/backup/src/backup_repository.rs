// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::{sink::BackupSink, source::BackupSource};

pub struct BackupRepository<Source: BackupSource, Sink: BackupSink> {
    pub source: Source,
    pub sink: Sink,
}

impl<Source: BackupSource, Sink: BackupSink> BackupRepository<Source, Sink> {
    pub fn writer(&self, backup_name: &str) -> Result<Sink::Writer, anyhow::Error> {
        self.sink.writer(backup_name)
    }

    pub fn reader(&self, backup_name: &str) -> Result<Source::Reader, anyhow::Error> {
        self.source.reader(backup_name)
    }
}
