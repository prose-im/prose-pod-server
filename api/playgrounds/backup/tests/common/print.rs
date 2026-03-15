// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use prose_backup::stats::ReadStats;

pub fn print_stats(
    raw_read_stats: &ReadStats,
    decryption_stats: &ReadStats,
    decompression_stats: &ReadStats,
    unarchived_size: u64,
) {
    tracing::info!("Stats:");
    tracing::info!("  Read:         {raw_read_stats}");
    tracing::info!("  Decrypted:    {decryption_stats}");
    tracing::info!("  Decompressed: {decompression_stats}");
    tracing::info!("  Unarchived:   {unarchived_size}B");

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
        "  Unarchiving:   {:.2}x",
        size_ratio(unarchived_size, &raw_read_stats)
    );
}
