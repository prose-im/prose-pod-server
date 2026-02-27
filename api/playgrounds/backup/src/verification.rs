// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Read;

use anyhow::{Context as _, anyhow};

use crate::{ProseBackupService, stores::ObjectStore};

impl<'s, S1, S2> ProseBackupService<'s, S1, S2>
where
    S1: ObjectStore,
    S2: ObjectStore,
{
    pub async fn download_backup_and_check_integrity(
        &self,
        backup_name: &crate::BackupFileName,
        created_at: impl Into<std::time::SystemTime>,
    ) -> Result<(tempfile::TempDir, std::path::PathBuf), anyhow::Error> {
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

        // TODO: Read backup to temporary file. It will have to be downloaded
        //   at some point anyway, and doing it this early allows us not to
        //   fetch it twice. It also allows us to easily performing integrity
        //   checks in parallel by opening multiple file descriptors (instead
        //   of writing a lot of overly complicated reading logic to reuse the
        //   same in-memory reader).
        // TODO: Run integrity checks.
        // TODO: Restore backup. Only extract backup AFTER running
        //   integrity checks to avoid potentially executing a malicious
        //   archive if it’s been tampered with.

        let todo = "Fix comment";
        // Read integrity checks in `Vec<u8>`s first. Avoids unnecessary
        // read of the whole backup file if something is wrong (i.e. fetch
        // fails, corrupted signature, no supported check…). Integrity
        // checks are quite small so loading all in memory is better than
        // saving to temporary files (less I/O).

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

// MARK: Fork reader

mod sha {
    use std::io::Read;

    use anyhow::anyhow;
    use sha2::{Digest as _, Sha256};

    use crate::util::to_hex;

    pub struct Sha256Check {
        hasher: Sha256,
        expected: [u8; 32],
    }

    impl Sha256Check {
        #[inline]
        pub fn new(expected: [u8; 32]) -> Self {
            debug_assert_eq!(expected.len(), Sha256::output_size());

            Self {
                hasher: Sha256::new(),
                expected,
            }
        }

        pub fn verify_reader<R: Read>(
            mut self: Box<Self>,
            reader: &mut R,
        ) -> Result<(), anyhow::Error> {
            std::io::copy(reader, &mut self.hasher);
            let result = self.hasher.finalize();
            if *result == self.expected {
                Ok(())
            } else {
                Err(anyhow!(
                    "Invalid hash. Got '0x{result}', expected '0x{expected}'.",
                    result = to_hex(result.as_ref()),
                    expected = to_hex(&self.expected)
                ))
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
                    MessageLayer::SignatureGroup { ref results } if i == 0 => {
                        if !results.iter().any(Result::is_ok) {
                            return Err(anyhow::anyhow!("No valid signature"));
                        }
                    }

                    layer => {
                        return Err(anyhow::anyhow!("Unexpected message structure ({layer:?})",));
                    }
                }
            }

            Ok(())
        }
    }
}
