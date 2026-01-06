// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Write;

use anyhow::Context as _;

use crate::{CreateBackupError, writer_chain::WriterChainBuilder};

#[derive(Debug)]
pub struct CompressionConfig {
    pub zstd_compression_level: i32,
}

impl<M, F> WriterChainBuilder<M, F> {
    pub(crate) fn compress<InnerWriter, OuterWriter>(
        self,
        config: &CompressionConfig,
    ) -> WriterChainBuilder<
        impl FnOnce(InnerWriter) -> Result<OuterWriter, CreateBackupError>,
        impl FnOnce(OuterWriter) -> Result<InnerWriter, CreateBackupError>,
    >
    where
        InnerWriter: Write,
        M: FnOnce(zstd::Encoder<'static, InnerWriter>) -> Result<OuterWriter, CreateBackupError>,
        F: FnOnce(OuterWriter) -> Result<zstd::Encoder<'static, InnerWriter>, CreateBackupError>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer: InnerWriter| {
                let writer = zstd::Encoder::new(writer, config.zstd_compression_level)
                    .context("Could not build zstd encoder")
                    .map_err(CreateBackupError::CannotCompress)?;

                make(writer)
            },

            finalize: move |writer: OuterWriter| {
                let writer = finalize(writer)?;

                let res = writer
                    .finish()
                    .map_err(anyhow::Error::new)
                    .map_err(CreateBackupError::CompressionFailed);

                res
            },
        }
    }
}
