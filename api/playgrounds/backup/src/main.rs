// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

extern crate sequoia_openpgp as openpgp;

use std::{
    fs::{self, File},
    io::{self, Read as _, Write as _},
    path::Path,
};

use anyhow::{Context as _, bail};
use bytes::Bytes;
use openpgp::parse::{Parse as _, stream::DecryptorBuilder};
use prose_backup::{ArchivingConfig, BackupService, CompressionConfig, EncryptionConfig};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let archive: tar::Archive<_> = tar::Archive::new(Default::default());

    let archiving_config = ArchivingConfig::new("./data");
    let compression_config = CompressionConfig {
        zstd_compression_level: 5,
    };
    let encryption_config = Some(EncryptionConfig::new(generate_test_cert()?));
    // let encryption_config = None;
    let integrity_config = Some(EncryptionConfig::new(generate_test_cert()?));
    // let integrity_config = None;

    let fs_prefix_backups = ".out/backups";
    fs::create_dir_all(fs_prefix_backups)?;
    let backup_store = prose_backup::stores::Fs::default()
        .overwrite(true)
        .directory(fs_prefix_backups);

    let fs_prefix_integrity_checks = ".out/integrity-checks";
    fs::create_dir_all(fs_prefix_integrity_checks)?;
    let integrity_check_store = prose_backup::stores::Fs::default()
        .overwrite(true)
        .directory(fs_prefix_integrity_checks);

    let service = BackupService {
        archiving_config,
        compression_config,
        encryption_config,
        integrity_config,
        backup_store,
        integrity_check_store,
    };

    let (backup_file_name, integrity_check_file_name) = {
        let backup_name = "backup";
        service.create_backup(backup_name, archive).await?
    };
    let backup_file_path = Path::new(fs_prefix_backups).join(&backup_file_name);
    let integrity_check_file_path =
        Path::new(fs_prefix_integrity_checks).join(&integrity_check_file_name);

    // Now checking.

    {
        let size_bytes = fs::metadata(&backup_file_path)?.len();
        println!("Backup size: {size_bytes}B");
    }

    // Integrity check
    service
        .check_backup_integrity(&backup_file_name, &integrity_check_file_name)
        .await
        .context("Integrity check failed")?;
    println!("Integrity check passed");

    {
        fn flip_one_bit_in_place(
            path: impl AsRef<Path>,
            pos: u64,
            mask: u8,
        ) -> std::io::Result<()> {
            use io::Seek as _;

            let mut f = fs::OpenOptions::new().read(true).write(true).open(path)?;

            // Read one byte
            let mut byte = [0u8];
            f.seek(io::SeekFrom::Start(pos))?;
            f.read_exact(&mut byte)?;

            // Flip bit
            byte[0] ^= mask;

            // Seek back and write new byte
            f.seek(io::SeekFrom::Start(pos))?;
            f.write_all(&byte)?;

            Ok(())
        }

        println!("Modifying integrity check…");
        flip_one_bit_in_place(&integrity_check_file_path, 10, 1 << 3)?;
        match service
            .check_backup_integrity(&backup_file_name, &integrity_check_file_name)
            .await
        {
            Err(err) => println!("Integrity check: {err:?} (expected)"),
            Ok(()) => bail!("Integrity check doesn’t work!"),
        }
    }

    let archive = if let Some(config) = service.encryption_config.as_ref() {
        let mut decryptor = DecryptorBuilder::from_file(&backup_file_path)?
            .with_policy(config.policy.as_ref(), None, config)
            .context("Could not decrypt archive")?;
        let mut buffer = Vec::new();
        decryptor.read_to_end(&mut buffer)?;
        let bytes = Bytes::from(buffer);
        let size_bytes = bytes.len();
        println!("Decrypted {size_bytes}B");

        bytes
    } else {
        eprintln!("NOT DECRYPTING");

        File::open(&backup_file_path)?
            .bytes()
            .collect::<Result<Bytes, std::io::Error>>()?
    };

    let archive = zstd::decode_all(&archive[..])?;
    println!("Decompressed {size_bytes}B", size_bytes = archive.len());

    print!("\n");
    print_tar_tree(&archive)?;

    print!("\n");
    let backups = service.list_backups().await?;
    println!("backups: {backups:?}");

    Ok(())
}

// MARK: - Helpers

fn print_tar_tree(data: &[u8]) -> Result<(), anyhow::Error> {
    let cursor = io::Cursor::new(data);
    let mut archive = tar::Archive::new(cursor);

    let mut size_bytes = 0;

    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        let size = entry.header().size()?;
        let entry_type = entry.header().entry_type();

        let type_char = match entry_type {
            tar::EntryType::Directory => 'd',
            tar::EntryType::Regular => 'f',
            tar::EntryType::Symlink => 'l',
            _ => '?',
        };

        println!("{} {:>6} {}", type_char, size, path.display());

        if let Ok(entry_size) = entry.header().entry_size() {
            size_bytes += entry_size;
        }
    }

    println!("Total unarchived: {size_bytes}B");

    Ok(())
}

fn generate_test_cert() -> Result<openpgp::Cert, anyhow::Error> {
    use openpgp::cert::CertBuilder;

    // Build a cert with user ID + primary key + subkey
    let (cert, _signature) = CertBuilder::new()
        .add_userid("Test User <test@example.org>")
        .add_signing_subkey()
        .add_storage_encryption_subkey()
        .set_validity_period(std::time::Duration::from_secs(3600))
        .generate()?;

    Ok(cert)
}
