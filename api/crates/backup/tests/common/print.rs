// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use prose_backup::stats::ReadStats;

use crate::common::prelude::DebugExtractBackupEventHandler;

pub fn print_stats(
    DebugExtractBackupEventHandler {
        raw_read_stats,
        decryption_stats,
        decompression_stats,
        extracted_bytes_count,
        ..
    }: &DebugExtractBackupEventHandler,
) {
    tracing::info!("Stats:");
    tracing::info!("  Read:         {raw_read_stats}");
    tracing::info!("  Decrypted:    {decryption_stats}");
    tracing::info!("  Decompressed: {decompression_stats}");
    tracing::info!("  Extracted:    {extracted_bytes_count}B");

    fn size_ratio(read: u64, reference: &ReadStats) -> f64 {
        let read: u32 = read.min(u64::from(u32::MAX)) as u32;
        let reference: u32 = reference.bytes_read.min(u64::from(u32::MAX)) as u32;
        f64::from(read) / f64::from(reference)
    }
    tracing::info!("Size ratios:");
    tracing::info!(
        "  Raw read:      {:.2}x",
        size_ratio(raw_read_stats.bytes_read, &raw_read_stats)
    );
    tracing::info!(
        "  Decryption:    {:.2}x",
        size_ratio(decryption_stats.bytes_read, &raw_read_stats)
    );
    tracing::info!(
        "  Decompression: {:.2}x",
        size_ratio(decompression_stats.bytes_read, &raw_read_stats)
    );
    tracing::info!(
        "  Extraction:    {:.2}x",
        size_ratio(*extracted_bytes_count, &raw_read_stats)
    );
}
