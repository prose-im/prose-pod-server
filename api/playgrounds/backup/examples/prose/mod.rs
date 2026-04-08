// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod api;
pub mod dashboard;

// MARK: - Helpers

pub(crate) fn init_tsks(fs_root: impl AsRef<std::path::Path>) -> Result<(), anyhow::Error> {
    use anyhow::Context as _;
    use openpgp::serialize::Serialize as _;
    use std::time::SystemTime;

    let fs_root = fs_root.as_ref();

    fn generate_test_cert() -> Result<openpgp::Cert, anyhow::Error> {
        use openpgp::cert::CertBuilder;
        use std::time::Duration;

        let created_at = SystemTime::now() - Duration::from_hours(3);
        let validity = Duration::from_hours(24);

        // Build a TSK with user ID + primary key + subkey
        let (tsk, _signature) = CertBuilder::new()
            .set_profile(openpgp::Profile::RFC9580)?
            .add_userid("Test User <test@example.org>")
            .set_creation_time(created_at)
            .set_validity_period(validity)
            .add_signing_subkey()
            .add_storage_encryption_subkey()
            .generate()?;
        tracing::debug!(
            "Created TSK `{tsk}` valid from {} to {}.",
            time::UtcDateTime::from(created_at),
            time::UtcDateTime::from(created_at + validity)
        );

        Ok(tsk)
    }

    let cert = generate_test_cert()?;

    let certs_path = fs_root.join("usr/share/prose/certs");
    std::fs::create_dir_all(&certs_path).context(format!("Dir: {certs_path:?}"))?;

    let cert_path = certs_path.join("example.tsk");
    let mut file = std::fs::File::create_new(&cert_path).context(format!("File: {cert_path:?}"))?;
    cert.as_tsk().serialize(&mut file)?;

    Ok(())
}

pub(crate) fn init_prose_config(fs_root: impl AsRef<std::path::Path>) -> Result<(), anyhow::Error> {
    use anyhow::Context as _;
    use std::io::Write as _;
    use toml::toml;

    let fs_root = fs_root.as_ref();

    let pgp_tsk_path = fs_root
        .join("usr/share/prose/certs/example.tsk")
        .display()
        .to_string();
    let pgp_tsk_path = pgp_tsk_path.as_str();

    // Run locally if passed `--local` (useful when offline).
    let storage_config = if std::env::args().skip(1).any(|arg| arg == "--local") {
        let directory = fs_root.join("_storage").display().to_string();
        std::fs::create_dir(&directory).unwrap();
        toml! {
            provider = "fs"
            fs.directory = directory
        }
    } else {
        toml! {
            provider = "s3"
        }
    };

    let config = toml! {
        backups.storage = storage_config

        // [backups.compression]
        // algorithm = "off"

        [backups.encryption]
        mode = "pgp"
        pgp.tsk = pgp_tsk_path

        [backups.signing]
        pgp.enabled = true
        pgp.tsk = pgp_tsk_path
    };

    let config_path = fs_root.join("etc/prose/prose.toml");
    let mut config_file =
        std::fs::File::create(&config_path).context(format!("File: {config_path:?}"))?;
    config_file
        .write_all(config.to_string().as_bytes())
        .context(format!("File: {config_path:?}"))?;

    Ok(())
}
