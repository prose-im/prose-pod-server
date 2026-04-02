// prose-pod-server
//
// Copyright: 2026, Claude Sonnet 4.6
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! WARN: This has largely been written by Claude Sonnet 4.6 on 2026-04-03.
//!   It had been improved for debugging purposes (errors) but has largely
//!   been left untouched.

use std::fs;
use std::path::Path;

use anyhow::Context as _;

fn round_up_512(n: u64) -> u64 {
    (n + 511) & !511
}

/// Returns the number of bytes this entry will contribute to the archive.
/// `stored_path` is the path as it will appear inside the archive.
fn entry_size(stored_path: &Path, metadata: &fs::Metadata) -> u64 {
    let mut size = 512; // header block

    if metadata.is_file() {
        size += round_up_512(metadata.len());
    }

    // GNU LongLink: emitted when the stored path exceeds 100 bytes
    let path_bytes = stored_path.as_os_str().len();
    if path_bytes > 100 {
        // One 512-byte LongLink header + padded path string
        size += 512 + round_up_512(path_bytes as u64 + 1); // +1 for null terminator
    }

    size
}

pub fn estimate_tar_size(paths: &[&Path]) -> anyhow::Result<u64> {
    let mut total = 0u64;
    for &path in paths {
        let metadata = path.metadata().context(format!("Path {path:?}"))?;
        if metadata.file_type().is_dir() {
            total += walk(path).context(format!("Walking {path:?}"))?;
        } else {
            total += entry_size(path, &metadata);
        }
    }
    Ok(total + 1024u64) // end-of-archive marker
}

fn walk(path: &Path) -> anyhow::Result<u64> {
    let mut total = 512u64; // header for the root dir entry itself
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let meta = entry.metadata().context(format!("Entry {entry_path:?}"))?;
        let stored_path = entry_path
            .strip_prefix(path.parent().unwrap_or(path))
            .unwrap_or(&entry_path);

        total += entry_size(stored_path, &meta);

        if meta.is_dir() {
            total += walk(&entry_path).context(format!("Walking {entry_path:?}"))?;
        }
    }
    Ok(total)
}
