// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;

use prose_backup::archiving::ExtractionReport;
use prose_backup::decryption::DecryptionReport;
use prose_backup::stats::{ReadStats, StreamStats};
use prose_backup::stores::ObjectId;
use prose_backup::{BackupId, CreateBackupEventHandler, RestoreBackupEventHandler};

/// A [`CreateBackupEventHandler`] which records all the information we might
/// need to debug.
#[derive(Debug, Default)]
pub struct DebugCreateBackupEventHandler {
    pub expected_archive_size: u64,
    pub effective_archive_size: u64,
    pub object_sizes: HashMap<ObjectId, u64>,
    pub upload_durations: Vec<(ObjectId, std::time::Duration)>,
}

impl CreateBackupEventHandler for DebugCreateBackupEventHandler {
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

/// An [`ExtractBackupEventHandler`] which records all the information we might
/// need to debug.
#[derive(Debug, Default)]
pub struct DebugExtractBackupEventHandler {
    pub raw_read_stats: ReadStats,
    pub decryption_report: DecryptionReport,
    pub decryption_stats: ReadStats,
    pub decompression_stats: ReadStats,
    pub extracted_bytes_count: u64,
}

impl RestoreBackupEventHandler for DebugExtractBackupEventHandler {
    fn on_restoration_progress(&mut self, _backup_id: &BackupId, len: usize) {
        self.raw_read_stats.record_chunk(len);
    }

    fn on_decryption_finished(
        &mut self,
        _backup_id: &BackupId,
        stats: ReadStats,
        report: DecryptionReport,
    ) {
        self.decryption_stats = stats;
        self.decryption_report = report;
    }

    fn on_decompression_finished(&mut self, _backup_id: &BackupId, stats: ReadStats) {
        self.decompression_stats = stats;
    }

    fn on_extraction_finished(&mut self, _backup_id: &BackupId, report: ExtractionReport) {
        self.extracted_bytes_count = report.extracted_bytes_count;
    }
}
