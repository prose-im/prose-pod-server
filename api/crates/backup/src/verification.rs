// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Verification logic.

use std::sync::Arc;

use crate::BackupService;
use crate::stores::{ObjectId, ReadObjectError, ReadSizedObjectError};
use crate::util::PathGuard;

pub(crate) use self::VerificationContext as Context;

/// Do not download OpenPGP signatures if larger than 2KiB.
/// FYI EdDSA+SHA512 signatures are 191 bytes so we’re being pretty safe here.
///
/// Because integrity checks are stored in S3 buckets, where data transfer
/// might be charged, it’s important to avoid downloading excessively large
/// files a malicious actor might have stored. We also prevent Denial of Service
/// if we stay stuck at downloading a very very large file.
const MAX_PGP_SIGNATURE_LENGTH: u64 = 2 * 1024;

#[non_exhaustive]
#[derive(Debug, Default)]
pub struct VerificationContext {
    pub pgp: Option<PgpVerificationContext>,
}

pub struct VerificationOutput {
    pub backup_path: Arc<PathGuard>,
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
    pub known_signing_keys: Vec<PgpSignatureReport>,
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
    #[inline]
    pub async fn download_backup_and_check_integrity(
        &self,
        backup_id: &crate::BackupId,
        created_at: impl Into<std::time::SystemTime>,
        report: &mut VerificationReport,
    ) -> Result<VerificationOutput, VerificationError> {
        use anyhow::{Context as _, anyhow};

        let backup_id = ObjectId::from(backup_id);

        // Make sure the backup exists.
        // Integrity checks cannot be deleted; checking this first avoids
        // unnecessary network calls (potentially billed) and computation.
        let mut backup_reader = match self.backup_store.cached_reader(&backup_id).await {
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

            let check_name = backup_id.with_extension("sig");

            // Read the signature.
            let signature: &Vec<u8> = {
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
                    Err(ReadSizedObjectError::ReadFailed(
                        err @ ReadObjectError::ObjectNotFound(_),
                    )) => {
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

                let mut signature: Vec<u8> = Vec::new();

                let signature_len = std::io::copy(&mut reader, &mut signature)
                    .context("Failed reading OpenPGP signature")
                    .map_err(VerificationError::Other)?;
                debug_assert_ne!(signature_len, 0);

                &*report.signature.insert(signature)
            };

            // Create the verifier, applying the policy at the
            // creation date of the backup.
            // NOTE: Validates the signature, which avoids reading the
            //   backup entirely if the signature itself is invalid.
            let mut verifier =
                pgp::PgpSignatureVerifier::new(context, signature.as_slice(), created_at.into())
                    .context(format!("Invalid OpenPGP signature: `{check_name}`"))
                    .map_err(VerificationError::InvalidSignature)?;

            // Verify the signature.
            let pgp_verification_res = verifier.verify_reader(&mut backup_reader);

            let pgp_report = verifier.report();
            report.known_signing_keys = pgp_report.known_signing_keys;

            match pgp_verification_res {
                Ok(()) => report.is_intact = true,
                Err(err) => {
                    if report.known_signing_keys.is_empty() {
                        tracing::debug!(
                            "All OpenPGP signing keys for `{check_name}` are untrusted. Falling back to integrity checks. (Source: {err:#})"
                        );

                        backup_reader = self
                            .backup_store
                            .persist_cache_and_rewind(backup_reader)
                            .await
                            .context("Failed saving cache")
                            .map_err(VerificationError::Other)?;

                        break 'pgp_sig;
                    } else {
                        return Err(VerificationError::InvalidSignature(err.context(format!(
                            "Invalid OpenPGP signature (verify): `{check_name}`"
                        ))));
                    }
                }
            }

            tracing::debug!("OpenPGP signature verified.");

            let backup_path = self.backup_store.persist_cache(backup_reader).await;

            // Don’t process any other integrity check.
            return Ok(VerificationOutput { backup_path });
        }

        // Ensure backup is signed if configuration enforces it.
        if self.signing_context.is_signing_mandatory {
            return Err(VerificationError::BackupNotSigned);
        }

        #[cfg(feature = "hashing-blake3")]
        'blake3_check: {
            let check_name = backup_id.with_extension("blake3");

            // Read stored hash.
            let expected_hash: Vec<u8> = {
                let reader = self
                    .check_store
                    .reader_if_not_too_large(&check_name, blake3::OUT_LEN as u64)
                    .await;

                let mut reader = match reader {
                    Ok(reader) => reader,
                    Err(err @ ReadSizedObjectError::ObjectTooLarge { .. }) => {
                        tracing::debug!(
                            "BLAKE3 checksum file `{check_name}` too large. Skipping. (Source: {err:#})"
                        );
                        break 'blake3_check;
                    }
                    Err(
                        err @ ReadSizedObjectError::ReadFailed(ReadObjectError::ObjectNotFound(_)),
                    ) => {
                        tracing::debug!(
                            "BLAKE3 checksum file `{check_name}` not found. Skipping. (Source: {err:#})"
                        );
                        break 'blake3_check;
                    }
                    Err(ReadSizedObjectError::ReadFailed(ReadObjectError::Other(err))) => {
                        return Err(VerificationError::Other(err.context(format!(
                            "Failed opening BLAKE3 checksum reader for `{check_name}`"
                        ))));
                    }
                };

                let mut expected_hash: Vec<u8> = Vec::new();
                let hash_len = std::io::copy(&mut reader, &mut expected_hash)
                    .context("Failed reading BLAKE3 checksum")
                    .map_err(VerificationError::Other)?;

                // Abort early if the hash is invalid.
                if hash_len != blake3::OUT_LEN as u64 {
                    return Err(VerificationError::InvalidChecksum(anyhow!(
                        "Invalid BLAKE3 checksum: `{check_name}`."
                    )));
                }

                expected_hash
            };

            // Compute the hash again.
            let computed_hash: blake3::Hash = {
                let mut verifier = blake3::Hasher::new();

                let copied = std::io::copy(&mut backup_reader, &mut verifier)
                    .context(format!("Failed reading backup: `{backup_id}`"))
                    .map_err(VerificationError::Other)?;
                debug_assert_ne!(copied, 0);

                let computed_hash = verifier.finalize();

                #[cfg(debug_assertions)]
                assert_ne!(computed_hash, blake3::Hasher::new().finalize());

                computed_hash
            };

            // Verify the checksum.
            if computed_hash.as_bytes() != expected_hash.as_slice() {
                return Err(VerificationError::InvalidChecksum(anyhow!(
                    "Invalid BLAKE3 checksum (verify): `{check_name}`."
                )));
            }

            tracing::debug!("BLAKE3 checksum verified.");
            report.is_intact = true;

            let backup_path = self.backup_store.persist_cache(backup_reader).await;

            // Don’t process any other integrity check.
            return Ok(VerificationOutput { backup_path });
        }

        #[cfg(feature = "hashing-sha2")]
        'sha256_check: {
            use sha2::{Digest as _, Sha256};

            let check_name = backup_id.with_extension("sha256");

            // Read stored hash.
            let expected_hash: Vec<u8> = {
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
                    Err(
                        err @ ReadSizedObjectError::ReadFailed(ReadObjectError::ObjectNotFound(_)),
                    ) => {
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

                let mut expected_hash: Vec<u8> = Vec::new();
                let hash_len = std::io::copy(&mut reader, &mut expected_hash)
                    .context("Failed reading SHA-256 checksum")
                    .map_err(VerificationError::Other)?;

                // Abort early if the hash is invalid.
                if hash_len != Sha256::output_size() as u64 {
                    return Err(VerificationError::InvalidChecksum(anyhow!(
                        "Invalid SHA-256 checksum: `{check_name}`."
                    )));
                }

                expected_hash
            };

            // Compute the hash again.
            let computed_hash: sha2::digest::Output<Sha256> = {
                let mut verifier = digest_io::IoWrapper(Sha256::new());

                // Read the backup to a temporary file, but also feed it to the
                // SHA-256 hasher in parallel.
                {
                    let copied = std::io::copy(&mut backup_reader, &mut verifier)
                        .context(format!("Failed reading backup: `{backup_id}`"))
                        .map_err(VerificationError::Other)?;
                    debug_assert_ne!(copied, 0);
                }

                let computed_hash = verifier.0.finalize();

                #[cfg(debug_assertions)]
                assert_ne!(computed_hash, Sha256::new().finalize());

                computed_hash
            };

            // Verify the checksum.
            if computed_hash.as_ref() != expected_hash {
                return Err(VerificationError::InvalidChecksum(anyhow!(
                    "Invalid SHA-256 checksum (verify): `{check_name}`."
                )));
            }

            tracing::debug!("SHA-256 checksum verified.");
            report.is_intact = true;

            let backup_path = self.backup_store.persist_cache(backup_reader).await;

            // Don’t process any other integrity check.
            return Ok(VerificationOutput { backup_path });
        }

        Err(VerificationError::Other(anyhow!(
            "Could not check the integrity of the backup."
        )))
    }
}

pub use self::pgp::*;
pub mod pgp {
    use std::{io, sync::Arc, time::SystemTime};

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

    pub struct PgpVerificationContext {
        pub certs: Arc<Vec<openpgp::Cert>>,
        pub policy: Box<dyn openpgp::policy::Policy>,
    }

    impl PgpVerificationContext {
        fn new_helper(&self) -> PgpVerificationHelper {
            PgpVerificationHelper {
                certs: Arc::clone(&self.certs),
                report: PgpVerificationReport::default(),
            }
        }
    }

    #[derive(Debug)]
    pub struct PgpVerificationHelper {
        pub certs: Arc<Vec<openpgp::Cert>>,
        pub report: PgpVerificationReport,
    }

    #[derive(Debug, Default)]
    pub struct PgpVerificationReport {
        pub known_signing_keys: Vec<PgpSignatureReport>,
        pub rejected_signatures_count: usize,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct PgpSignatureReport {
        /// Fingerprint of the certificate containing the signing subkey.
        pub cert_fingerprint: openpgp::Fingerprint,

        /// Fingerprint of the signing subkey.
        pub subkey_fingerprint: Option<openpgp::Fingerprint>,

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
                                Ok(GoodChecksum { ka, .. }) => {
                                    tracing::trace!("Found signature: Ok(GoodChecksum {{ .. }})");

                                    self.report.known_signing_keys.push(PgpSignatureReport {
                                        cert_fingerprint: ka.cert().fingerprint(),
                                        subkey_fingerprint: Some(ka.key().fingerprint()),
                                        is_valid: true,
                                    });
                                }

                                Err(err) => {
                                    self.report.rejected_signatures_count =
                                        self.report.rejected_signatures_count.saturating_add(1);

                                    match &err {
                                        VerificationError::MissingKey { .. } => {
                                            tracing::trace!(
                                                "Found signature: Err(VerificationError::MissingKey {{ .. }})"
                                            );
                                        }

                                        VerificationError::UnboundKey { cert, .. } => {
                                            tracing::trace!(
                                                "Found signature: Err(VerificationError::UnboundKey {{ .. }})"
                                            );

                                            self.report.known_signing_keys.push(
                                                PgpSignatureReport {
                                                    cert_fingerprint: cert.fingerprint(),
                                                    subkey_fingerprint: None,
                                                    is_valid: false,
                                                },
                                            );
                                        }

                                        VerificationError::BadKey { ka, error, .. } => {
                                            tracing::trace!(
                                                "Found signature: Err(VerificationError::BadKey {{ error: \"{error:#}\", .. }})"
                                            );

                                            self.report.known_signing_keys.push(
                                                PgpSignatureReport {
                                                    cert_fingerprint: ka.cert().fingerprint(),
                                                    subkey_fingerprint: Some(
                                                        ka.key().fingerprint(),
                                                    ),
                                                    is_valid: false,
                                                },
                                            );
                                        }

                                        VerificationError::BadSignature { ka, error, .. } => {
                                            tracing::trace!(
                                                "Found signature: Err(VerificationError::BadSignature {{ error: \"{error:#}\", .. }})"
                                            );

                                            self.report.known_signing_keys.push(
                                                PgpSignatureReport {
                                                    cert_fingerprint: ka.cert().fingerprint(),
                                                    subkey_fingerprint: Some(
                                                        ka.key().fingerprint(),
                                                    ),
                                                    is_valid: false,
                                                },
                                            );
                                        }

                                        VerificationError::MalformedSignature { error, .. } => {
                                            tracing::trace!(
                                                "Found signature: Err(VerificationError::MalformedSignature {{ error: \"{error:#}\", .. }})"
                                            );
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

    impl std::fmt::Debug for PgpVerificationContext {
        #[rustfmt::skip]
        #[inline]
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let Self { certs, policy } = self;

            f.debug_struct("PgpVerificationContext")
                .field("certs", &certs.iter().map(openpgp::Cert::fingerprint).collect::<Vec<_>>())
                .field("policy", policy)
                .finish()
        }
    }
}
