// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::{sink::BackupSink, source::BackupSource};

pub struct BackupRepository<Source: BackupSource, Sink: BackupSink> {
    pub backup_source: Source,
    pub backup_sink: Sink,
}
