// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::{ProseBackupService, stores::ObjectStore};

impl<'s, S1, S2> ProseBackupService<'s, S1, S2>
where
    S1: ObjectStore,
    S2: ObjectStore,
{
    /// Reads backup into a temporary file, then runs integrity checks on it.
    ///
    /// If the backup is intact and complies with the configured security
    /// policy, this method returns a path to the local file. Otherwise,
    /// the backup is deleted.
    ///
    /// Note that while we try to do everything in a streaming manner, when
    /// restoring a backup it would actually be dangerous to extract the backup
    /// at the same time as validating its signature/digest. Indeed, archives
    /// can be crafted to exploit a bug in the decompression library for
    /// example. For this reason we MUST validate the authenticity of the
    /// backup first, and only then proceed to processing it.
    ///
    /// Downloading to a file prevents downloading the backup twice, which
    /// might be charged depending on where it is stored. By being frugal
    /// we also benefit from a very fast second read, as the data is already
    /// on disk. We could also run mutliple integrity checks in parallel by
    /// opening multiple file descriptors (instead of writing a lot of overly
    /// complicated reading logic to reuse the same in-memory reader).
    ///
    /// Integrity checks are read before the backup is downloaded, to avoid
    /// an unnecessary read of the whole backup file if something is wrong
    /// (e.g. network error, corrupted signature, no authenticity proof…).
    /// They are saved in memory because they are relatively small (few
    /// hundred bytes). No need to save it to temporary files, it would only
    /// add I/O overhead.
    pub async fn download_backup_and_check_integrity(
        &self,
        backup_name: &crate::BackupFileName,
        created_at: impl Into<std::time::SystemTime>,
    ) -> Result<(tempfile::TempDir, std::path::PathBuf), anyhow::Error> {
        use anyhow::{Context as _, anyhow};
        use std::io::Read as _;

        // Open local file paths.
        // If permissions are not sufficient, avoids unnecessary network
        // calls (potentially billed).
        let tmp = tempfile::TempDir::new()
            .context("Failed creating a temporary directory to download the backup in")?;
        let backup_path = tmp.path().join(&backup_name);
        let mut backup_file = std::fs::File::create_new(&backup_path)
            .context("Failed opening a file path to download the backup to")?;

        // Make sure the backup exists.
        // Integrity checks cannot be deleted; checking this first avoids
        // unnecessary network calls (potentially billed) and computation.
        let Some(mut backup_reader) = (self.backup_store)
            .reader(&backup_name)
            .await
            .context("Failed opening backup reader")?
        else {
            return Err(anyhow!("Backup not found"));
        };

        let fixme = "Fetch max number of bytes from S3, to avoid DoS";
        // const MAX_SIGNATURE_SIZE: usize = 2 * 1024; // 2KiB

        // Look for an OpenPGP signature.
        if let Some(context) = self.pgp_verification_context.as_ref() {
            let check_name = backup_name.with_extension("sig");

            let reader = self.check_store.reader(&check_name).await.context(format!(
                "Failed opening integrity check reader for `{check_name}`"
            ))?;

            if let Some(mut reader) = reader {
                // Read signature.
                let mut bytes: Vec<u8> = Vec::new();
                reader.read_to_end(&mut bytes);

                // Create the verifier, applying the policy at the
                // creation date of the backup.
                // NOTE: Validates the signature, which avoids reading the
                //   backup entirely if the signature itself is invalid.
                let mut verifier =
                    pgp::PgpSignatureVerifier::new(context.to_owned(), &bytes, created_at.into())
                        .context(format!("Invalid OpenPGP signature: `{check_name}`"))?;

                // Read the backup to a temporary file.
                std::io::copy(&mut backup_reader, &mut backup_file)?;

                // Verify the signature.
                verifier.verify_reader(&mut backup_file).context(format!(
                    "Invalid OpenPGP signature (verify): `{check_name}`"
                ))?;

                tracing::debug!("OpenPGP signature verified.");

                // Don’t process any other integrity check.
                return Ok((tmp, backup_path));
            } else {
                tracing::info!(
                    "Found OpenPGP signature file `{check_name}` but cannot read it because of missing configuration. Skipping."
                )
            }
        }

        // Ensure backup is signed if configuration enforces it.
        if self.signing_config.as_ref().is_some_and(|c| c.mandatory) {
            return Err(anyhow!(
                "Backup not signed (but signing is mandatory per configuration)."
            ));
        }

        {
            let check_name = backup_name.with_extension("sha256");

            let reader = self.check_store.reader(&check_name).await.context(format!(
                "Failed opening integrity check reader for `{check_name}`"
            ))?;

            if let Some(mut reader) = reader {
                todo!("Check hash.");
            } else {
                return Err(anyhow!("Could not check the integrity of the backup."));
            }
        }
    }
}

pub mod pgp {
    use std::{io, time::SystemTime};

    use anyhow::anyhow;
    use openpgp::parse::{Parse as _, stream::*};

    #[repr(transparent)]
    pub struct PgpSignatureVerifier<'sig, 'cert>(
        DetachedVerifier<'sig, PgpVerificationHelper<'cert>>,
    );

    impl<'sig, 'cert> PgpSignatureVerifier<'sig, 'cert> {
        pub fn new<'policy: 'sig>(
            context: &'_ PgpVerificationContext<'cert, 'policy>,
            expected: &'sig [u8],
            time: SystemTime,
        ) -> Result<Self, anyhow::Error> {
            let verifier = DetachedVerifierBuilder::from_bytes(expected)?.with_policy(
                context.policy,
                Some(time),
                context.helper.clone(),
            )?;

            Ok(Self(verifier))
        }

        pub fn verify_reader<R: io::Read + Send + Sync>(
            &mut self,
            reader: &mut R,
        ) -> Result<(), anyhow::Error> {
            self.0.verify_reader(reader)
        }
    }

    #[derive(Debug)]
    pub struct PgpVerificationContext<'cert, 'policy> {
        pub helper: PgpVerificationHelper<'cert>,
        pub policy: &'policy dyn openpgp::policy::Policy,
    }

    #[derive(Debug, Clone)]
    pub struct PgpVerificationHelper<'cert> {
        pub cert: &'cert openpgp::Cert,
    }

    impl<'cert> VerificationHelper for PgpVerificationHelper<'cert> {
        fn get_certs(
            &mut self,
            _ids: &[openpgp::KeyHandle],
        ) -> Result<Vec<openpgp::Cert>, anyhow::Error> {
            let fixme = "Return multiple certs";

            Ok(vec![self.cert.clone()])
        }

        fn check(&mut self, structure: MessageStructure) -> Result<(), anyhow::Error> {
            for (i, layer) in structure.into_iter().enumerate() {
                match layer {
                    MessageLayer::SignatureGroup { results } if i == 0 => {
                        if !results.iter().any(Result::is_ok) {
                            return Err(anyhow!("No valid signature."));
                        }
                    }

                    layer => {
                        return Err(anyhow!("Unexpected message structure ({layer:?})."));
                    }
                }
            }

            Ok(())
        }
    }
}
