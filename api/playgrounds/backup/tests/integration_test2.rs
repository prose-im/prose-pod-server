// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{error::Error, fs, path::Path};

use prose_backup::{ArchivingConfig, BackupService, CompressionConfig, source::FileSource};

use crate::temp_file_sink::TempFileSink;

mod temp_file_sink;

#[test]
fn backup_enc_none_sig_hash() -> Result<(), Box<dyn Error>> {
    let archiving_config = ArchivingConfig::new("./data");
    let compression_config = CompressionConfig {
        zstd_compression_level: 5,
    };
    let encryption_config = None;
    let integrity_config = None;

    let tmp_dir = Path::new(".test").join(function_name_short!());
    fs::create_dir_all(&tmp_dir)?;
    let sink = TempFileSink::new(&tmp_dir);
    let source = FileSource::new(&tmp_dir);
    let service = BackupService {
        archiving_config,
        compression_config,
        encryption_context: encryption_config,
        hashing_config: integrity_config,
        sink,
        source,
    };

    let (backup_file_name, integrity_check_file_name) = {
        let backup_name = "backup";
        service.create_backup(backup_name)?
    };

    Ok(())
}

#[test]
fn backup_enc_none_sig_gpg() {}

#[test]
fn backup_enc_gpg_hash() {}

#[test]
fn backup_enc_gpg_sig_gpg() {}

#[test]
fn backup_enc_none_sig_gpg_withpass() {}

#[test]
fn backup_enc_gpg_withpass_sig_hash() {}

macro_rules! function_name {
    () => {{
        fn f() {}
        // Get the fully-qualified path, then strip the trailing "::f"
        let name = std::any::type_name_of_val(&f);
        &name[..name.len() - 3]
    }};
}
use function_name;

macro_rules! function_name_short {
    () => {{
        let full = function_name!();
        full.rsplit("::").next().unwrap()
    }};
}
use function_name_short;
