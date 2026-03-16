// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Compression logic.

use std::io::Write;

use anyhow::Context as _;

use crate::{config::CompressionConfig, writer_chain::WriterChainBuilder};

use self::errors::*;

pub mod errors {
    #[derive(Debug, thiserror::Error)]
    #[error("Cannot compress")]
    #[repr(transparent)]
    pub struct CannotCompress(#[from] pub anyhow::Error);

    #[derive(Debug, thiserror::Error)]
    #[error("Compression failed")]
    #[repr(transparent)]
    pub struct CompressionFailed(#[from] pub anyhow::Error);
}

pub(crate) fn compress<'a, W: Write>(
    config: &CompressionConfig,
) -> WriterChainBuilder<
    impl FnOnce(W) -> Result<zstd::Encoder<'a, W>, CannotCompress>,
    impl FnOnce(zstd::Encoder<'a, W>) -> Result<W, CompressionFailed>,
> {
    WriterChainBuilder {
        make: move |writer: W| {
            zstd::Encoder::new(writer, config.zstd_compression_level)
                .context("Could not build zstd encoder")
                .map_err(CannotCompress)
        },

        finalize: move |writer: zstd::Encoder<'a, W>| {
            writer
                .finish()
                .map_err(anyhow::Error::new)
                .map_err(CompressionFailed)
        },
    }
}
