// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Compression logic.

use std::io::Write;

use anyhow::Context as _;
use composable_stream::ComposableStreamBuilder;

use crate::CreateBackupError;
use crate::config::CompressionConfig;

pub(crate) fn compress<'a, W: Write>(
    config: &CompressionConfig,
) -> ComposableStreamBuilder<
    impl FnOnce(W) -> Result<zstd::Encoder<'a, W>, CreateBackupError>,
    impl FnOnce(zstd::Encoder<'a, W>) -> Result<W, CreateBackupError>,
> {
    ComposableStreamBuilder {
        make: move |writer: W| {
            zstd::Encoder::new(writer, config.zstd_compression_level)
                .context("Could not build zstd encoder")
                .map_err(CreateBackupError::CannotCompress)
        },

        finalize: move |writer: zstd::Encoder<'a, W>| {
            writer
                .finish()
                .map_err(anyhow::Error::new)
                .map_err(CreateBackupError::CompressionFailed)
        },
    }
}
