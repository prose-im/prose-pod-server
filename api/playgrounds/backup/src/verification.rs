// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Verification logic.

use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use crate::BackupService;
use crate::stores::{ReadObjectError, ReadSizedObjectError};

pub use self::VerificationContext as Context;

/// Do not download OpenPGP signatures if larger than 2KiB.
/// FYI it should be 191 bytes so we’re being pretty safe here.
///
/// Because integrity checks are stored in S3 buckets, where data transfer
/// might be charged, it’s important to avoid downloading excessively large
/// files a malicious actor might have stored. We also prevent Denial of Service
/// if we stay stuck at downloading a very very large file.
const MAX_PGP_SIGNATURE_LENGTH: u64 = 2 * 1024;

#[non_exhaustive]
#[derive(Default)]
pub struct VerificationContext {
    pub pgp: Option<PgpVerificationContext>,
}

pub struct VerificationOutput {
    pub tmp_dir: tempfile::TempDir,
    pub backup_path: std::path::PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    #[error("Backup not found")]
    BackupNotFound(#[source] anyhow::Error),

    #[error(transparent)]
    InvalidSignature(anyhow::Error),

    #[error("Backup not signed (but signing is mandatory per configuration).")]
    BackupNotSigned,

    #[error(transparent)]
    InvalidChecksum(anyhow::Error),

    #[error(transparent)]
    Other(anyhow::Error),
}

#[derive(Debug, Default)]
pub struct VerificationReport {
    pub is_signed: bool,
    pub is_encrypted: bool,
    pub can_be_restored: bool,
    pub is_intact: bool,
    pub signing_keys: Vec<PgpSignatureReport>,
    pub signature: Option<Vec<u8>>,
    pub is_encryption_valid: bool,
}

impl BackupService {
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
        report: &mut VerificationReport,
    ) -> Result<VerificationOutput, VerificationError> {
        use anyhow::{Context as _, anyhow};
        use std::io::Read as _;

        // Open local file paths.
        // If permissions are not sufficient, avoids unnecessary network
        // calls (potentially billed).
        let tmp_dir = tempfile::TempDir::new()
            .context("Failed creating a temporary directory to download the backup in")
            .map_err(VerificationError::Other)?;
        let backup_path = tmp_dir.path().join(&backup_name);

        let mut backup_file = std::fs::File::options()
            // Allow creating the file and writing to it.
            .create_new(true)
            .write(true)
            // Allow reading the file (necessary when verifying).
            .read(true)
            // Only allow read and write for the current user.
            // This is very important, as not doing so would virtually leak all
            // Prose data if the backup is unencrypted (default mode is `644`).
            .mode(0o600)
            .open(&backup_path)
            .context("Failed opening a file path to download the backup to")
            .map_err(VerificationError::Other)?;
        if cfg!(debug_assertions) {
            let metadata = std::fs::metadata(&backup_path).unwrap();
            debug_assert_eq!(metadata.permissions().mode(), 0o100600);
        }

        // Make sure the backup exists.
        // Integrity checks cannot be deleted; checking this first avoids
        // unnecessary network calls (potentially billed) and computation.
        let mut backup_reader = match self.backup_store.reader(&backup_name).await {
            Ok(reader) => reader,
            Err(ReadObjectError::ObjectNotFound(err)) => {
                return Err(VerificationError::BackupNotFound(err));
            }
            Err(ReadObjectError::Other(err)) => {
                return Err(VerificationError::Other(
                    err.context("Failed opening backup reader"),
                ));
            }
        };

        // Look for an OpenPGP signature.
        'pgp_sig: {
            let Some(context) = self.verification_context.pgp.as_ref() else {
                tracing::debug!("OpenPGP signature not checked: Missing configuration.");
                break 'pgp_sig;
            };

            let check_name = backup_name.with_extension("sig");

            let reader = self
                .check_store
                .reader_if_not_too_large(&check_name, MAX_PGP_SIGNATURE_LENGTH)
                .await;

            let mut reader = match reader {
                Ok(reader) => reader,
                Err(err @ ReadSizedObjectError::ObjectTooLarge { .. }) => {
                    tracing::debug!(
                        "OpenPGP signature file `{check_name}` too large. Skipping. (Source: {err:#})"
                    );
                    break 'pgp_sig;
                }
                Err(ReadSizedObjectError::ReadFailed(err @ ReadObjectError::ObjectNotFound(_))) => {
                    tracing::debug!(
                        "OpenPGP signature file `{check_name}` not found. Skipping. (Source: {err:#})"
                    );
                    break 'pgp_sig;
                }
                Err(ReadSizedObjectError::ReadFailed(ReadObjectError::Other(err))) => {
                    return Err(VerificationError::Other(err.context(format!(
                        "Failed opening OpenPGP signature reader for `{check_name}`"
                    ))));
                }
            };
            report.is_signed = true;

            // Read signature.
            let mut signature: Vec<u8> = Vec::new();
            reader
                .read_to_end(&mut signature)
                .context("Failed reading OpenPGP signature")
                .map_err(VerificationError::Other)?;
            let signature: &Vec<u8> = &*report.signature.insert(signature);

            // Create the verifier, applying the policy at the
            // creation date of the backup.
            // NOTE: Validates the signature, which avoids reading the
            //   backup entirely if the signature itself is invalid.
            let mut verifier =
                pgp::PgpSignatureVerifier::new(context, &signature, created_at.into())
                    .context(format!("Invalid OpenPGP signature: `{check_name}`"))
                    .map_err(VerificationError::InvalidSignature)?;

            // Read the backup to a temporary file.
            std::io::copy(&mut backup_reader, &mut backup_file)
                .context(format!("Failed reading backup: `{backup_name}`"))
                .map_err(VerificationError::Other)?;

            // Verify the signature.
            verifier
                .verify_reader(&mut backup_file)
                .context(format!(
                    "Invalid OpenPGP signature (verify): `{check_name}`"
                ))
                .map_err(VerificationError::InvalidSignature)?;
            report.is_intact = true;

            let pgp_report = verifier.report();
            report.signing_keys = pgp_report.signing_keys;

            tracing::debug!("OpenPGP signature verified.");

            // Don’t process any other integrity check.
            return Ok(VerificationOutput {
                tmp_dir,
                backup_path,
            });
        }

        // Ensure backup is signed if configuration enforces it.
        if self.signing_context.is_signing_mandatory {
            return Err(VerificationError::BackupNotSigned);
        }

        'sha256_check: {
            use crate::writer_chain::tee::TeeWriter;
            use sha2::{Digest as _, Sha256};

            let check_name = backup_name.with_extension("sha256");

            let reader = self
                .check_store
                .reader_if_not_too_large(&check_name, Sha256::output_size() as u64)
                .await;

            let mut reader = match reader {
                Ok(reader) => reader,
                Err(err @ ReadSizedObjectError::ObjectTooLarge { .. }) => {
                    tracing::debug!(
                        "SHA-256 checksum file `{check_name}` too large. Skipping. (Source: {err:#})"
                    );
                    break 'sha256_check;
                }
                Err(err @ ReadSizedObjectError::ReadFailed(ReadObjectError::ObjectNotFound(_))) => {
                    tracing::debug!(
                        "SHA-256 checksum file `{check_name}` not found. Skipping. (Source: {err:#})"
                    );
                    break 'sha256_check;
                }
                Err(ReadSizedObjectError::ReadFailed(ReadObjectError::Other(err))) => {
                    return Err(VerificationError::Other(err.context(format!(
                        "Failed opening SHA-256 checksum reader for `{check_name}`"
                    ))));
                }
            };

            // Read signature.
            let mut expected_hash: Vec<u8> = Vec::new();
            reader
                .read_to_end(&mut expected_hash)
                .context("Failed reading SHA-256 checksum")
                .map_err(VerificationError::Other)?;

            // Create the verifier and abort early if the hash is invalid.
            if expected_hash.len() != Sha256::output_size() {
                return Err(VerificationError::InvalidChecksum(anyhow!(
                    "Invalid SHA-256 checksum: `{check_name}`."
                )));
            }
            let verifier = Sha256::new();

            // Read the backup to a temporary file, but also feed it to the
            // SHA-256 hasher in parallel.
            let mut tee_writer = TeeWriter::new(backup_file, verifier);
            std::io::copy(&mut backup_reader, &mut tee_writer)
                .context(format!("Failed reading backup: `{backup_name}`"))
                .map_err(VerificationError::Other)?;

            let TeeWriter {
                // NOTE: It’s okay to drop the file as-is:
                //   the OS will flush before closing the file.
                w1: _backup_file,
                w2: verifier,
            } = tee_writer;

            // Verify the checksum.
            if verifier.finalize().as_ref() == expected_hash {
                tracing::debug!("SHA-256 checksum verified.");
                report.is_intact = true;

                // Don’t process any other integrity check.
                return Ok(VerificationOutput {
                    tmp_dir,
                    backup_path,
                });
            } else {
                return Err(VerificationError::InvalidChecksum(anyhow!(
                    "Invalid SHA-256 checksum (verify): `{check_name}`."
                )));
            }
        }

        Err(VerificationError::Other(anyhow!(
            "Could not check the integrity of the backup."
        )))
    }
}

pub use self::pgp::*;
pub mod pgp {
    use std::{io, rc::Rc, time::SystemTime};

    use anyhow::anyhow;
    use openpgp::parse::{Parse as _, stream::*};

    #[repr(transparent)]
    pub struct PgpSignatureVerifier<'ctx>(DetachedVerifier<'ctx, PgpVerificationHelper>);

    impl<'ctx> PgpSignatureVerifier<'ctx> {
        pub fn new<'sig: 'ctx>(
            context: &'ctx PgpVerificationContext,
            expected: &'sig [u8],
            time: SystemTime,
        ) -> Result<Self, anyhow::Error> {
            let verifier = DetachedVerifierBuilder::from_bytes(expected)?.with_policy(
                context.policy.as_ref(),
                Some(time),
                context.new_helper(),
            )?;

            Ok(Self(verifier))
        }

        pub fn verify_reader<R: io::Read + Send + Sync>(
            &mut self,
            reader: &mut R,
        ) -> Result<(), anyhow::Error> {
            self.0.verify_reader(reader)
        }

        pub fn report(self) -> PgpVerificationReport {
            self.0.into_helper().report
        }
    }

    #[derive(Debug)]
    pub struct PgpVerificationContext {
        pub certs: Rc<Vec<openpgp::Cert>>,
        pub policy: Box<dyn openpgp::policy::Policy>,
    }

    impl PgpVerificationContext {
        fn new_helper(&self) -> PgpVerificationHelper {
            PgpVerificationHelper {
                certs: Rc::clone(&self.certs),
                report: PgpVerificationReport::default(),
            }
        }
    }

    #[derive(Debug)]
    pub struct PgpVerificationHelper {
        pub certs: Rc<Vec<openpgp::Cert>>,
        pub report: PgpVerificationReport,
    }

    #[derive(Debug, Default)]
    pub struct PgpVerificationReport {
        pub signing_keys: Vec<PgpSignatureReport>,
    }

    #[derive(Debug)]
    pub struct PgpSignatureReport {
        pub cert_fingerprint: openpgp::Fingerprint,
        pub subkey_fingerprint: Option<openpgp::Fingerprint>,
        pub is_trusted: bool,
        pub is_valid: bool,
    }

    impl VerificationHelper for PgpVerificationHelper {
        fn get_certs(
            &mut self,
            _ids: &[openpgp::KeyHandle],
        ) -> Result<Vec<openpgp::Cert>, anyhow::Error> {
            Ok(Vec::clone(&self.certs))
        }

        fn check(&mut self, structure: MessageStructure) -> Result<(), anyhow::Error> {
            for (i, layer) in structure.into_iter().enumerate() {
                match layer {
                    MessageLayer::SignatureGroup { results } if i == 0 => {
                        let mut errors = Vec::new();

                        for result in results {
                            match result {
                                Ok(checksum) => {
                                    self.report.signing_keys.push(PgpSignatureReport {
                                        cert_fingerprint: checksum.ka.cert().fingerprint(),
                                        subkey_fingerprint: Some(checksum.ka.key().fingerprint()),
                                        is_trusted: true,
                                        is_valid: true,
                                    });
                                }

                                Err(err) => {
                                    match &err {
                                        VerificationError::UnboundKey { cert, .. } => {
                                            self.report.signing_keys.push(PgpSignatureReport {
                                                cert_fingerprint: cert.fingerprint(),
                                                subkey_fingerprint: None,
                                                is_trusted: true,
                                                is_valid: false,
                                            });
                                        }

                                        VerificationError::BadKey { ka, .. }
                                        | VerificationError::BadSignature { ka, .. } => {
                                            self.report.signing_keys.push(PgpSignatureReport {
                                                cert_fingerprint: ka.cert().fingerprint(),
                                                subkey_fingerprint: Some(ka.key().fingerprint()),
                                                is_trusted: true,
                                                is_valid: false,
                                            });
                                        }

                                        err @ VerificationError::MissingKey { sig }
                                        | err @ VerificationError::MalformedSignature {
                                            sig, ..
                                        } => {
                                            let mut fingerprints =
                                                sig.issuer_fingerprints().peekable();
                                            if fingerprints.peek().is_none() {
                                                tracing::warn!(
                                                    "Signature has no issuer fingerprint. Issuers: {issuers:?}. Error: {err:?}",
                                                    issuers = sig.get_issuers()
                                                );
                                            }
                                            for fingerprint in fingerprints {
                                                self.report.signing_keys.push(PgpSignatureReport {
                                                    cert_fingerprint: fingerprint.clone(),
                                                    subkey_fingerprint: None,
                                                    is_trusted: false,
                                                    is_valid: false,
                                                });
                                            }
                                        }

                                        err => {
                                            tracing::warn!("{err:?}");
                                        }
                                    };
                                    errors.push(err)
                                }
                            }
                        }

                        if !errors.is_empty() {
                            if cfg!(debug_assertions) {
                                tracing::debug!(
                                    "SignatureGroup errors: {:#?}",
                                    errors
                                        .into_iter()
                                        .map(|err| err.to_string())
                                        .collect::<Vec<_>>()
                                );
                            }
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
