// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

// TODO: Test permissions after unpacking.
// TODO: Test unpackiing doesn’t override other directories.
// TODO: Test backups are atomic (no deletion then ).

#[test]
fn test_backup_creates_valid_zstd() -> std::io::Result<()> {
    // Path for the dummy file
    let dummy_path = Path::new("data/var/lib/prosody/upload%2eprose%2elocal/example.bin");

    // Create dummy file with 10 MB of NULL bytes
    create_dummy_file(dummy_path, 10 * 1024 * 1024)?;

    // Run your library's backup function
    // Assuming `prose_backup::run_backup()` is the entry point for backups
    prose_backup::run_backup().expect("Backup failed");

    // Path to the backup file
    let backup_path = Path::new("backup.tar.zst");
    assert!(backup_path.exists(), "Backup file was not created");

    // Validate that it's a valid zstd file
    let mut backup_file = File::open(backup_path)?;
    let mut decoder =
        Decoder::new(&mut backup_file).expect("Backup file is not a valid Zstandard stream");

    // Try reading the decompressed contents (even if we don't need them)
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;

    // Optional: check that decompressed size matches the dummy file size
    assert!(decompressed.len() >= 10 * 1024 * 1024);

    // Clean up after test
    fs::remove_file(dummy_path)?;
    fs::remove_file(backup_path)?;

    Ok(())
}

#[test]
fn test_integrity_is_checked() {
    // Create backup.
    todo!();

    // Ensure backup is valid.
    todo!();

    // Flip a bit in integrity check.
    todo!();
    // {
    //     fn flip_one_bit_in_place(
    //         path: impl AsRef<Path>,
    //         pos: u64,
    //         mask: u8,
    //     ) -> std::io::Result<()> {
    //         use io::Seek as _;

    //         let mut f = fs::OpenOptions::new().read(true).write(true).open(path)?;

    //         // Read one byte
    //         let mut byte = [0u8];
    //         f.seek(io::SeekFrom::Start(pos))?;
    //         f.read_exact(&mut byte)?;

    //         // Flip bit
    //         byte[0] ^= mask;

    //         // Seek back and write new byte
    //         f.seek(io::SeekFrom::Start(pos))?;
    //         f.write_all(&byte)?;

    //         Ok(())
    //     }

    //     tracing::info!("Modifying integrity check…");
    //     flip_one_bit_in_place(&integrity_check_file_path, 10, 1 << 3)?;
    //     match service
    //         .check_backup_integrity(&backup_file_name, &integrity_check_file_name)
    //         .await
    //     {
    //         Err(err) => tracing::info!("Integrity check: {err:?} (expected)"),
    //         Ok(()) => bail!("Integrity check doesn’t work!"),
    //     }
    // }

    // Check backup cannot be restored.
    todo!();
}

#[test]
fn test_attacker_cannot_remove_integrity_check() {
    todo!()
}

#[test]
fn test_authenticity_check_cannot_be_bypassed() {
    todo!()
}

/// Ensures that the default backup config does not work.
///
/// Imagine if someone configures encryption but makes a typo in the key and
/// they end up storing unencrypted backups without knowing because the default
/// config is applied. That’d be bad.
#[test]
fn test_default_config_does_not_work() {
    todo!()
}

#[test]
fn test_encryption_is_disabled_by_default() {
    todo!()
}

#[test]
fn test_gpg_encryption_needs_a_secret_key() {
    // Load config with encryption enabled but no secret key defined.
    todo!();

    // Check that creating a backup fails.
    todo!();
}

/// Make sure no data is output at all when preconditions fail.
///
/// When streaming data to a network sink (e.g. a S3 bucket), we don’t want to
/// upload a partial backup header if preconditions fail. That’d result in
/// corrupted backups. Instead we want to try our best not to get partial
/// uploads (although faults can always happen).
#[test]
fn test_no_output_at_all_if_precondition_failed() {
    todo!();
}

#[test]
fn test_listing_backups_returns_metadata() {
    // List objects in backups bucket.
    // For a given page, list objects in checks bucket that match
    //   StartAfter = name of the first backup
    //   Then stop when one name is > (name of the last backup).zzz
    //     (NOTE: In ASCII, z > Z so it’s safe)

    // signing <- config
    // encryption <- config
    // is_intact = integrity check
    //
    // is_encryption_valid = is_encryption_correct && is_encryption_key_recognized
    // WARN: If `!signing.mandatory` then `!is_signature_trusted` is ignored!
    // is_trusted = true
    //   && (!signing.enabled || !signing.mandatory || (is_signed && is_signature_trusted))
    //   && (!encryption.enabled || !encryption.mandatory || (is_encrypted && is_encryption_valid))
    // can_be_restored = is_intact && is_trusted
    //
    // BackupDto {
    //   id: "hash(backup_name)",
    //   metadata: {
    //     name: "2025-12-16 automatic backup",
    //     created_at: 2025-12-16T15:49:35Z,
    //     is_intact: true,
    //     is_signed: true,
    //     is_signature_valid: true,
    //     is_encrypted: true,
    //     is_encryption_valid: true,
    //     is_trusted: true,
    //     can_be_restored: true,
    //   },
    //   restore_id: "prose-backup-2025-12-16",
    // }
    // BackupDto {
    //   id: "hash(backup_name)",
    //   metadata: {
    //     name: "2025-12-16 automatic backup",
    //     created_at: 2025-12-16T15:59:35Z,
    //     is_intact: true,
    //     is_signed: false,
    //     is_encrypted: true,
    //     is_trusted: false,
    //     can_be_restored: false,
    //   },
    // }

    todo!();
}

#[test]
fn test_listing_backups_tells_if_backup_corrupted() {
    todo!();
}

#[test]
fn test_restoring_backup_requires_backup_not_corrupted() {
    todo!();
}

#[test]
fn test_tls_certs_are_not_backed_up() {
    todo!();
}

#[test]
fn test_tls_certs_can_be_backed_up() {
    todo!();
}

#[test]
fn test_tls_certs_cannot_be_backed_up_if_encryption_is_disabled() {
    todo!();
}

/// Need checksum when enabling signing.
#[test]
fn test_metadata_returns_checksum() {}
