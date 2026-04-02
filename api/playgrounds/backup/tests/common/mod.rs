// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#![allow(dead_code, unused_imports, unused_macros)]

pub mod blueprints;
pub mod fs;
pub mod lifecycle;
pub mod pgp;
pub mod print;
#[cfg(feature = "provider_s3")]
pub mod s3;

#[allow(unused_imports)]
pub mod prelude {
    pub use std::collections::HashMap;
    pub use std::path::{Path, PathBuf};
    pub use std::sync::Arc;
    pub use std::time::{Duration, SystemTime};

    pub use anyhow::{Context as _, anyhow};
    pub use prose_backup::archiving::{ArchiveBlueprint, ArchivingContext};
    pub use prose_backup::config::*;
    pub use prose_backup::decryption::PgpDecryptionContext;
    pub use prose_backup::{
        BackupConfig, BackupId, BackupService, CreateBackupCommand, CreateBackupOutput,
        CreateBackupSuccess, ExtractAndRestoreSuccess, ExtractionSuccess,
    };
    pub use toml::toml;

    pub use super::blueprints::*;
    pub use super::fs::*;
    pub use super::lifecycle::*;
    pub(crate) use super::macros::*;
    pub use super::pgp::*;
    pub use super::run_command;
    pub use super::unique_hex;
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

mod macros {
    macro_rules! env_required {
        ($name:literal) => {
            std::env::var($name).expect(concat!(
                "Environment variable `",
                $name,
                "` should be defined"
            ))
        };
    }
    pub(crate) use env_required;

    macro_rules! log_error {
        () => {
            #[inline]
            |error| tracing::error!("{:#}", error)
        };
    }
    pub(crate) use log_error;
}

pub fn run_command(subject: impl std::fmt::Display, command: &mut std::process::Command) {
    let output = command.output().unwrap();

    let status = output.status;

    let mut output_str = Vec::with_capacity(output.stdout.len() + output.stderr.len());
    output_str.extend(output.stdout);
    output_str.extend(output.stderr);

    tracing::debug!("{subject}:\n{}", String::from_utf8(output_str).unwrap());

    assert!(status.success(), "{status:#}");
}
