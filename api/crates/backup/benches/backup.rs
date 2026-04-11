// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use prose_backup::archiving::{AdditionalData, ArchiveBlueprint, TarSizeCalculator};
use prose_backup::config::HashingAlgorithm;
use prose_backup::event_handlers::NoopEventHandler;
use prose_backup::{BackupService, CreateBackupCommand, CreateBackupSuccess};
use tempfile::TempPath;

use std::hint::black_box;
use std::io::{self, Read as _};
use std::time::Duration;

use crate::common::*;

#[rustfmt::skip]
const FILE_SIZE_TEST_CASES: [u64; 4] = [
//            1 * 1024, //   1KiB
//           16 * 1024, //  16KiB
//          128 * 1024, // 128KiB
//     1 * 1024 * 1024, //   1MiB
      16 * 1024 * 1024, //  16MiB
     128 * 1024 * 1024, // 128MiB
     512 * 1024 * 1024, // 512MiB
    1024 * 1024 * 1024, //   1GiB
];

#[rustfmt::skip]
const FILE_COUNT_TEST_CASES: [u32; 3] = [
           16,
     8 * 1024,
    16 * 1024,
];

fn test_data_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("benchmark-data")
}

fn test_store_path() -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("benchmark-store");

    if !path.is_dir() {
        std::fs::create_dir_all(&path).unwrap();
    }

    path
}

fn bench_file_size_no_file_io(c: &mut Criterion) {
    let mut group = c.benchmark_group("backup_file_size_no_file_io");

    // Lower sample size as what we’re measuring is quite long to execute.
    group.sample_size(10);

    // No need to warm up for 3 seconds (default).
    group.warm_up_time(Duration::from_secs(1));

    struct AdditionalFiles {
        file_count: u32,
        file_size: u64,
    }

    impl AdditionalData for AdditionalFiles {
        fn expected_size(&self) -> Result<u64, anyhow::Error> {
            let entry_size = TarSizeCalculator::file_entry_size("foo/n", self.file_size);
            Ok((self.file_count as u64) * entry_size)
        }

        fn append<W: std::io::Write>(
            self,
            builder: &mut tar::Builder<W>,
        ) -> Result<(), anyhow::Error> {
            let Self {
                mut file_count,
                file_size,
            } = self;

            while file_count > 0 {
                let reader = black_box(io::repeat(0).take(file_size));

                let mut header = tar::Header::new_gnu();
                header.set_size(file_size);
                header.set_cksum();

                builder.append_data(&mut header, format!("foo/{file_count}"), reader)?;

                file_count -= 1;
            }

            Ok(())
        }
    }

    async fn benchmark(service: &BackupService, file_count: u32, file_size: u64) {
        let command = CreateBackupCommand {
            prefix: "bench",
            description: "Benchmark",
            blueprint: &ArchiveBlueprint {
                version: 1,
                paths: vec![],
            },
            additional_archive_data: Some(AdditionalFiles {
                file_count,
                file_size,
            }),
            #[cfg(feature = "test")]
            created_at: std::time::SystemTime::now(),
        };

        service
            .create_backup(command, &mut NoopEventHandler)
            .await
            .unwrap();
    }

    let file_count = 1;
    let zstd_compression_level = 3;

    for file_size in FILE_SIZE_TEST_CASES {
        group.throughput(Throughput::Bytes(file_count as u64 * file_size));

        group.bench_with_input(
            BenchmarkId::new("blake3", &file_size),
            &file_size,
            |b, &file_size| {
                let service = sinking_service(zstd_compression_level, HashingAlgorithm::Blake3);

                b.to_async(tokio_runtime())
                    .iter(|| benchmark(&service, file_count, black_box(file_size)))
            },
        );

        group.bench_with_input(
            BenchmarkId::new("sha256", &file_size),
            &file_size,
            |b, &file_size| {
                let service = sinking_service(zstd_compression_level, HashingAlgorithm::Sha256);

                b.to_async(tokio_runtime())
                    .iter(|| benchmark(&service, file_count, black_box(file_size)))
            },
        );
    }

    group.finish();
}

fn bench_file_size(c: &mut Criterion) {
    let mut group = c.benchmark_group("backup_file_size_with_file_io");

    // Lower sample size as what we’re measuring is quite long to execute.
    group.sample_size(10);

    // No need to warm up for 3 seconds (default).
    group.warm_up_time(Duration::from_secs(2));

    let file_count = 1;
    let zstd_compression_level = 3;
    let test_data_path = test_data_path();
    let test_store_path = test_store_path();

    for file_size in FILE_SIZE_TEST_CASES {
        let files_path = init_files(file_count, file_size, &test_data_path);
        let blueprint = ArchiveBlueprint::new(1, [("foo", files_path)]);

        group.throughput(Throughput::Bytes(file_count as u64 * file_size));

        group.bench_function(BenchmarkId::new("blake3", &file_size), |b| {
            let service = fs_service(
                zstd_compression_level,
                HashingAlgorithm::Blake3,
                &test_store_path,
            );

            b.to_async(tokio_runtime()).iter_with_large_drop(|| {
                benchmark_create_backup(&service, &blueprint, &test_store_path)
            })
        });

        group.bench_function(BenchmarkId::new("sha256", &file_size), |b| {
            let service = fs_service(
                zstd_compression_level,
                HashingAlgorithm::Sha256,
                &test_store_path,
            );

            b.to_async(tokio_runtime()).iter_with_large_drop(|| {
                benchmark_create_backup(&service, &blueprint, &test_store_path)
            })
        });
    }

    group.finish();
}

fn bench_file_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("backup_file_count_with_file_io");

    // Lower sample size as what we’re measuring is quite long to execute.
    group.sample_size(10);

    // No need to warm up for 3 seconds (default).
    group.warm_up_time(Duration::from_secs(2));

    let total_file_size: u64 = 128 * 1024 * 1024; // 128MiB.
    let zstd_compression_level = 3;
    let test_data_path = test_data_path();
    let test_store_path = test_store_path();

    for file_count in FILE_COUNT_TEST_CASES {
        let file_size = total_file_size / file_count as u64;

        let files_path = init_files(file_count, file_size, &test_data_path);
        let blueprint = ArchiveBlueprint::new(1, [("foo", files_path)]);

        group.throughput(Throughput::Bytes(total_file_size));

        group.bench_function(BenchmarkId::new("blake3", &file_count), |b| {
            let service = fs_service(
                zstd_compression_level,
                HashingAlgorithm::Blake3,
                &test_store_path,
            );

            b.to_async(tokio_runtime()).iter_with_large_drop(|| {
                benchmark_create_backup(&service, &blueprint, &test_store_path)
            })
        });

        group.bench_function(BenchmarkId::new("sha256", &file_count), |b| {
            let service = fs_service(
                zstd_compression_level,
                HashingAlgorithm::Sha256,
                &test_store_path,
            );

            b.to_async(tokio_runtime()).iter_with_large_drop(|| {
                benchmark_create_backup(&service, &blueprint, &test_store_path)
            })
        });
    }

    group.finish();
}

async fn benchmark_create_backup(
    service: &BackupService,
    blueprint: &ArchiveBlueprint,
    test_store_path: impl AsRef<std::path::Path>,
) -> Vec<TempPath> {
    let command: CreateBackupCommand = CreateBackupCommand {
        prefix: "bench",
        description: &format!("Benchmark {}", unique_hex().unwrap()),
        blueprint,
        additional_archive_data: None,
        #[cfg(feature = "test")]
        created_at: std::time::SystemTime::now(),
    };

    let CreateBackupSuccess { output, .. } = service
        .create_backup(command, &mut NoopEventHandler)
        .await
        .unwrap();

    // Create path guards for created files, so it’s cleaned up when the
    // function result is dropped (avoids measuring the deletion as part
    // of the benchmark).
    let mut object_guards: Vec<TempPath> = Vec::new();
    let test_store_path = test_store_path.as_ref();
    object_guards
        .push(TempPath::try_from_path(test_store_path.join(output.backup_id.to_string())).unwrap());
    for id in output.digest_ids {
        object_guards.push(TempPath::try_from_path(test_store_path.join(id)).unwrap());
    }
    for id in output.signature_ids {
        object_guards.push(TempPath::try_from_path(test_store_path.join(id)).unwrap());
    }
    object_guards
}

criterion_group!(
    benches,
    bench_file_size_no_file_io,
    bench_file_size,
    bench_file_count
);
criterion_main!(benches);

fn tokio_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}
