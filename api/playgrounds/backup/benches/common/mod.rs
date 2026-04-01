// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#![allow(dead_code)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use prose_backup::{
    BackupService,
    archiving::ArchivingContext,
    config::{CachingConfig, CompressionConfig, DownloadConfig, HashingAlgorithm, HashingConfig},
    decryption::DecryptionContext,
    signing::SigningContext,
    stores::{CachedStore, FsStore, StoreCache},
    verification::VerificationContext,
};
use tokio::sync::RwLock;

use crate::common::sink_store::SinkStore;

pub mod sink_store;

/// A `BackupService` which dismisses data (no persistence).
pub fn sinking_service(
    zstd_compression_level: i32,
    hashing_algorithm: HashingAlgorithm,
) -> BackupService {
    BackupService {
        archiving_context: ArchivingContext {
            blueprints: HashMap::new(),
        },
        compression_config: CompressionConfig {
            zstd_compression_level,
        },
        hashing_config: HashingConfig {
            algorithm: hashing_algorithm,
        },
        encryption_context: None,
        signing_context: SigningContext::default(),
        verification_context: VerificationContext::default(),
        decryption_context: DecryptionContext::default(),
        download_config: DownloadConfig {
            url_max_ttl: std::time::Duration::ZERO,
        },
        backup_store: CachedStore::new(
            Box::new(SinkStore),
            Arc::new(RwLock::new(StoreCache::default())),
            &CachingConfig {
                cache_dir: tempfile::env::temp_dir(),
                max_backup_cache_size: None,
            },
        ),
        check_store: Box::new(SinkStore),
    }
}

/// A `BackupService` which stores data on the file system.
pub fn fs_service(
    zstd_compression_level: i32,
    hashing_algorithm: HashingAlgorithm,
    path: impl AsRef<Path>,
) -> BackupService {
    let store = FsStore {
        directory: path.as_ref().to_path_buf(),
        overwrite: false,
        mode: 0o600,
    };

    BackupService {
        archiving_context: ArchivingContext {
            blueprints: HashMap::new(),
        },
        compression_config: CompressionConfig {
            zstd_compression_level,
        },
        hashing_config: HashingConfig {
            algorithm: hashing_algorithm,
        },
        encryption_context: None,
        signing_context: SigningContext::default(),
        verification_context: VerificationContext::default(),
        decryption_context: DecryptionContext::default(),
        download_config: DownloadConfig {
            url_max_ttl: std::time::Duration::ZERO,
        },
        backup_store: CachedStore::new(
            Box::new(store.clone()),
            Arc::new(RwLock::new(StoreCache::default())),
            &CachingConfig {
                cache_dir: tempfile::env::temp_dir(),
                max_backup_cache_size: None,
            },
        ),
        check_store: Box::new(store),
    }
}

// NOTE: Implementation cannot be time-based, even with nanosecond precision,
//   as tests are ran concurrently and such conflicts happen (very often).
//   When it does, one test cleaning up its temporary directory causes another
//   to fail. We don’t want that.
pub fn unique_hex() -> Result<String, std::io::Error> {
    use std::io::Read as _;

    let mut urandom = std::fs::File::open("/dev/urandom")?;
    let mut buf = [0u8; 4]; // 4 bytes = 8 hex chars
    urandom.read_exact(&mut buf)?;

    let hex = buf.iter().map(|b| format!("{:02x}", b)).collect();

    Ok(hex)
}

#[must_use]
pub fn init_files(
    mut file_count: u32,
    file_size: u64,
    test_data_path: impl AsRef<Path>,
) -> PathBuf {
    // Compute the number of `1MiB` blocks `dd` must create.
    let block_size = (1024 * 1024).min(file_size);
    if file_size.rem_euclid(block_size) != 0 {
        panic!("Invalid file_size: `{file_size}` must be a multiple of `{block_size}`.");
    }
    let block_count = file_size.div_euclid(block_size);

    let dir_path = test_data_path
        .as_ref()
        .join(format!("{file_count}x{file_size}"));
    if dir_path.is_dir() {
        eprintln!("{dir_path:?} already exists, skipping.");
        return dir_path;
    } else {
        std::fs::create_dir_all(&dir_path).unwrap();
    }

    eprintln!("Creating {file_count} file(s) of {file_size}B in {dir_path:?}…");

    // TODO: Use `xargs` for parallelism?
    while file_count > 0 {
        let dd_status = std::process::Command::new("dd")
            .arg("if=/dev/urandom")
            .arg(format!("of={file_count}"))
            .arg(format!("bs={block_size}"))
            .arg(format!("count={block_count}"))
            .current_dir(&dir_path)
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(dd_status.success());

        file_count -= 1;
    }

    dir_path
}
