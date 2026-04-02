// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;

use prose_backup::{BackupId, CreateBackupEventHandler, stores::ObjectId};

/// An event handler which records all the information we might need to debug.
#[derive(Debug, Default)]
pub struct DebugEventHandler {
    pub expected_archive_size: u64,
    pub effective_archive_size: u64,
    pub object_sizes: HashMap<ObjectId, u64>,
    pub upload_durations: Vec<(ObjectId, std::time::Duration)>,
}

impl CreateBackupEventHandler for DebugEventHandler {
    fn on_archive_start(&mut self, _backup_id: &BackupId, expected_archive_size: u64) {
        tracing::debug!("Expected archive size: {expected_archive_size}");
        self.expected_archive_size = expected_archive_size;
    }

    fn on_archive_progress(&mut self, _backup_id: &BackupId, archived_bytes: usize) {
        self.effective_archive_size += archived_bytes as u64;
    }

    fn on_upload_progress(&mut self, object_id: &ObjectId, uploaded_bytes: usize) {
        *self.object_sizes.entry(object_id.clone()).or_default() += uploaded_bytes as u64;
    }

    fn on_backup_uploaded(
        &mut self,
        backup_id: &BackupId,
        _size_bytes: u64,
        duration: std::time::Duration,
    ) {
        self.upload_durations
            .push((ObjectId::from(backup_id), duration));
    }

    fn on_digest_uploaded(&mut self, object_id: &ObjectId, duration: std::time::Duration) {
        self.upload_durations.push((object_id.clone(), duration));
    }

    fn on_signature_uploaded(&mut self, object_id: &ObjectId, duration: std::time::Duration) {
        self.upload_durations.push((object_id.clone(), duration));
    }
}
