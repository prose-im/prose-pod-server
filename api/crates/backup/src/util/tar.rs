// prose-pod-server
//
// Copyright: 2026, Claude Sonnet 4.6
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context as _;

const BLOCK_SIZE: u64 = 512;
const ARCHIVE_HEADER_SIZE: u64 = 0u64;
const ENTRY_HEADER_SIZE: u64 = BLOCK_SIZE;
const END_OF_ARCHIVE_MARKER_SIZE: u64 = 2 * BLOCK_SIZE;
const LONG_LINK_HEADER_SIZE: u64 = BLOCK_SIZE;

#[non_exhaustive]
pub struct TarSizeCalculator;

impl TarSizeCalculator {
    fn entry_header_size(path_in_archive: impl AsRef<OsStr>) -> u64 {
        let mut header_size = ENTRY_HEADER_SIZE;

        // GNU LongLink: Emitted when the stored path exceeds 100 bytes.
        let path_bytes = path_in_archive.as_ref().len();
        if path_bytes > 100 {
            let path_len = path_bytes as u64 + 1; // +1 for NULL terminator
            header_size += LONG_LINK_HEADER_SIZE + round_up_blocks(path_len);

            tracing::trace!("Added long link header size.");
        }

        header_size
    }

    /// Returns the number of bytes this entry will contribute to the archive.
    /// `stored_path` is the path as it will appear inside the archive.
    pub fn file_entry_size(path_in_archive: impl AsRef<OsStr>, expected_len: u64) -> u64 {
        let mut entry_size = Self::entry_header_size(path_in_archive);

        entry_size += round_up_blocks(expected_len);

        entry_size
    }

    /// Returns the number of bytes this entry will contribute to the archive.
    /// `stored_path` is the path as it will appear inside the archive.
    pub fn entry_size_at_path(path_in_archive: impl AsRef<OsStr>, metadata: &fs::Metadata) -> u64 {
        let mut entry_size = Self::entry_header_size(path_in_archive.as_ref());

        if metadata.is_file() {
            let file_len = metadata.len();
            entry_size += round_up_blocks(file_len);
        }

        entry_size
    }

    /// Estimates a `tar` archive’s total size given the paths which will be
    /// archived.
    pub fn estimate_tar_size<'a, S: AsRef<OsStr> + 'a, P: AsRef<Path> + 'a>(
        paths: impl IntoIterator<Item = &'a (S, P)>,
    ) -> anyhow::Result<u64> {
        let mut total = ARCHIVE_HEADER_SIZE;

        for (key, path) in paths {
            let path = path.as_ref();

            let metadata = path.metadata().context(format!("Path {path:?}"))?;
            if metadata.file_type().is_dir() {
                total += Self::walk(path, Path::new(key)).context(format!("Walking {path:?}"))?;
            } else {
                total += Self::entry_size_at_path(key, &metadata);
            }
        }

        Ok(total + END_OF_ARCHIVE_MARKER_SIZE)
    }

    pub fn archive_contents_size(archive_len: u64) -> u64 {
        let overhead = ARCHIVE_HEADER_SIZE + END_OF_ARCHIVE_MARKER_SIZE;

        debug_assert!(
            archive_len > overhead,
            "Archive too small: {archive_len} <= {overhead}"
        );

        archive_len.saturating_sub(overhead)
    }

    fn walk(reference: &Path, dst: &Path) -> Result<u64, anyhow::Error> {
        let mut total = 0u64;

        let mut paths = VecDeque::from([reference.to_path_buf()]);

        while let Some(path) = paths.pop_front() {
            let meta = path.metadata().context(format!("Path {path:?}"))?;
            let path_in_archive = dst.join(path.strip_prefix(reference).unwrap());

            total += Self::entry_size_at_path(path_in_archive, &meta);

            if meta.is_dir() {
                for entry in std::fs::read_dir(&path)? {
                    let entry = entry?;
                    paths.push_back(entry.path());
                }
            }
        }

        Ok(total)
    }
}

fn round_up_blocks(n: u64) -> u64 {
    (n + (BLOCK_SIZE - 1)) & !(BLOCK_SIZE - 1)
}
