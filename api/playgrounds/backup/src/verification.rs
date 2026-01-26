// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::{self, Read, Write};

use anyhow::{Context as _, anyhow};
use openpgp::parse::{
    Parse,
    stream::{DetachedVerifier, DetachedVerifierBuilder},
};
use sha2::{Digest as _, Sha256};

use crate::{ProseBackupService, stores::ObjectStore};

pub(crate) struct IntegrityCheckDescriptor<'a> {
    pub suffix: &'a str,

    pub value: Vec<u8>,
}

pub(crate) struct IntegrityCheckBuilder;

impl IntegrityCheckBuilder {
    pub fn new(
        // Suffix after the backup file name (e.g. `.sig`, `.sha256`…).
        suffix: &str,
    ) -> Result<impl Fn(Vec<u8>) -> IntegrityCheck, anyhow::Error> {
        match suffix {
            ".sig" => Ok(IntegrityCheck::PgpSignature),
            ".sha256" => Ok(IntegrityCheck::Sha256),
            suffix => Err(anyhow!("Unrecognized integrity check suffix: `{suffix}`.")),
        }
    }
}

pub(crate) enum IntegrityCheck {
    Sha256(Vec<u8>),
    PgpSignature(Vec<u8>),
}

impl IntegrityCheck {
    fn pre_validate(&self) -> Result<(), anyhow::Error> {
        match self {
            Self::PgpSignature(value) => todo!(),
            Self::Sha256(hash) => {
                let hash_len = hash.len();
                if hash_len != 32 {
                    return Err(anyhow!("SHA-256 hash has incorrect length: {hash_len}"));
                }
            }
        }
        Ok(())
    }
}

pub fn pre_validate_integrity_checks(checks: &[IntegrityCheck]) -> Result<(), anyhow::Error> {
    for check in checks {
        check.pre_validate()?;
    }

    Ok(())
}

#[non_exhaustive]
pub struct VerificationHelper<'a> {
    gpg: Option<&'a self::pgp::PgpVerificationHelper>,
}

impl<'s, S1, S2> ProseBackupService<'s, S1, S2>
where
    S1: ObjectStore,
    S2: ObjectStore,
{
    pub async fn check_backup_integrity<R: Read>(
        &self,
        backup_name: &str,
        backup_reader_builder: impl Fn() -> R,
    ) -> Result<(), anyhow::Error> {
        use std::io::Read as _;

        let integrity_checks = (self.check_store)
            .find(backup_name)
            .await
            .context("Failed listing integrity checks")?;

        if integrity_checks.is_empty() {
            return Err(anyhow!(
                "No integrity check stored for '{backup_name}'. \
                Cannot check integrity."
            ));
        }

        let mut backup_reader = self
            .backup_store
            .reader(backup_name)
            .await
            .context("Could not open backup reader")?;

        let mut integrity_checked = false;

        for integrity_check in integrity_checks.iter() {
            match integrity_check
                .strip_prefix(backup_name)
                .unwrap_or(integrity_check.as_str())
            {
                ".sig" => {
                    let Some(helper) = self.verification_helper.gpg else {
                        tracing::debug!(
                            "Cannot check '{integrity_check}': OpenPGP keys not configured."
                        );
                        continue;
                    };

                    let mut verifier = DetachedVerifierBuilder::from_reader(backup_reader)?;
                    std::io::copy(&mut backup_reader, &mut verifier.writer)
                        .context("Could not read backup")?;

                    let mut integrity_check_reader = self
                        .check_store
                        .reader(integrity_check)
                        .await
                        .context("Could not open integrity check reader")?;
                    let mut integrity_check: Vec<u8> = Vec::new();
                    integrity_check_reader
                        .read_to_end(&mut integrity_check)
                        .context("Could not read integrity check")?;

                    verifier.verify(&integrity_check)?;
                }
                ".sha256" => todo!(),
            }

            tracing::debug!("Integrity check passed ({integrity_check}).");

            integrity_checked = true;
        }

        if integrity_checked {
            Ok(backup_reader)
        } else {
            Err(anyhow!(
                "No integrity check could be processed for '{backup_name}'. \
                Integrity check failed."
            ))
        }
    }
}

pub type BackupVerifier = ProseBackupVerifier;

pub(crate) struct ProseBackupVerifier {
    pub(crate) writer: IntegrityChecker,
}

impl BackupVerifier {
    pub(crate) fn new(integrity_config: &IntegrityWriterBuilder) -> Self {
        let writer = match integrity_config {
            IntegrityWriterBuilder::Sha256 => todo!(),
        };
        Self { writer }
    }

    pub(crate) fn verify(self, integrity_check: &Vec<u8>) -> Result<(), anyhow::Error> {
        self.writer.verify(integrity_check)
    }
}

#[non_exhaustive]
pub enum IntegrityChecker {
    /// Integrity only.
    Sha256 { hasher: Sha256 },
}

impl IntegrityChecker {
    fn new(integrity_config: &IntegrityWriterBuilder) -> Self {
        match integrity_config {
            IntegrityWriterBuilder::Sha256 => Self::Sha256 {
                hasher: Sha256::new(),
            },
        }
    }
}

impl Write for IntegrityChecker {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Sha256 { hasher, .. } => hasher.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Sha256 { hasher, .. } => hasher.flush(),
        }
    }
}

impl IntegrityChecker {
    fn verify(self, integrity_check: &[u8]) -> Result<(), anyhow::Error> {
        match self {
            Self::Sha256 { hasher } => {
                let hash = hasher.finalize();
                if integrity_check == hash.to_vec().as_slice() {
                    Ok(())
                } else {
                    Err(anyhow!("Invalid hash."))
                }
            }
        }
    }
}

// MARK: Fork reader

pub trait IntegrityCheck<'a, R: Read>: Write + Send {
    fn verify_reader(self: Box<Self>, reader: &mut R) -> Result<(), anyhow::Error>;
}

pub struct ForkReader<'a, R> {
    reader: R,
    checks: Vec<Box<dyn IntegrityCheck<'a, R> + 'a>>,
}

impl<'a, R: Read> ForkReader<'a, R> {
    /// Create a new ForkReader with the given reader
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            checks: Vec::new(),
        }
    }

    /// Add an integrity check
    pub fn add_check(&mut self, check: Box<dyn IntegrityCheck<'a> + 'a>) {
        self.checks.push(check);
    }

    /// Consume the reader and return results of all checks
    pub fn verify(mut self) -> io::Result<Vec<bool>> {
        // Read through the entire file
        let mut buffer = vec![0u8; 8192];
        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    // Update all checks with this chunk
                    for check in &mut self.checks {
                        check.update(&buffer[..n]);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }

        // Finalize all checks and collect results
        Ok(self.checks.into_iter().map(|c| c.finalize()).collect())
    }
}

impl<'a, R: Read> Read for ForkReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(buf)?;

        // Update all checks with the data that was read
        for check in &mut self.checks {
            check.update(&buf[..n]);
        }

        Ok(n)
    }
}

mod sha {
    use std::io::{self, Read, Write};

    use anyhow::anyhow;
    use sha2::{Digest as _, Sha256};

    use crate::util::to_hex;

    pub struct Sha256Check {
        hasher: Sha256,
        expected: [u8; 32],
    }

    impl Sha256Check {
        pub fn new(expected: [u8; 32]) -> Self {
            Self {
                hasher: Sha256::new(),
                expected,
            }
        }
    }

    impl Write for Sha256Check {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.hasher.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.hasher.flush()
        }
    }

    impl<'a, R: Read> super::IntegrityCheck<'a, R> for Sha256Check {
        fn verify_reader(mut self: Box<Self>, reader: &mut R) -> Result<(), anyhow::Error> {
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

mod pgp {
    use std::{
        fs::File,
        io::{self, Write},
        time::SystemTime,
    };

    use openpgp::parse::{
        Parse,
        stream::{DetachedVerifier, DetachedVerifierBuilder, MessageLayer, MessageStructure},
    };

    pub struct PgpSignatureCheck<'data, 'cert> {
        verifier: DetachedVerifier<'data, PgpVerificationHelper<'cert>>,
    }

    impl<'data, 'cert> PgpSignatureCheck<'data, 'cert> {
        pub fn new(
            policy: &'data dyn openpgp::policy::Policy,
            helper: PgpVerificationHelper<'cert>,
            expected: &'data [u8],
            time: SystemTime,
        ) -> Result<Self, anyhow::Error> {
            let verifier = DetachedVerifierBuilder::from_bytes(expected)?.with_policy(
                policy,
                Some(time),
                helper,
            )?;
            Ok(Self { verifier })
        }
    }

    impl<'data, 'cert> PgpSignatureCheck<'data, 'cert> {
        fn finalize(self) -> Result<(), anyhow::Error> {
            self.verifier.build()?.finalize()
        }
    }

    impl<'data, 'cert> Write for PgpSignatureCheck<'data, 'cert> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.signer.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.signer.flush()
        }
    }

    impl<'data, 'cert, R: io::Read + Send + Sync> super::IntegrityCheck<'cert, R>
        for PgpSignatureCheck<'data, 'cert>
    {
        fn verify_reader(mut self: Box<Self>, reader: &mut R) -> Result<(), anyhow::Error> {
            self.verifier.verify_reader(reader)
        }
    }

    #[non_exhaustive]
    pub struct SignatureVerifier<'data, 'helper> {
        pgp: DetachedVerifier<'data, PgpVerificationHelper<'helper>>,
    }

    // impl<'data, 'helper> SignatureVerifier<'data, 'helper> {
    //     pub(crate) fn new(integrity_config: &SignatureWriterBuilder) -> Self {
    //         let writer = match integrity_config {
    //             SignatureWriterBuilder::Gpg(helper) => todo!(),
    //         };
    //         Self { writer }
    //     }

    //     pub(crate) fn verify(self, integrity_check: &Vec<u8>) -> Result<(), anyhow::Error> {
    //         self.writer.verify(integrity_check)
    //     }
    // }

    #[derive(Debug)]
    pub struct PgpVerificationHelper<'cert> {
        cert: &'cert openpgp::Cert,
    }

    impl<'cert> openpgp::parse::stream::VerificationHelper for PgpVerificationHelper<'cert> {
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
