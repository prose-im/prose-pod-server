// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Compression logic.

use std::io::Write;
use std::marker::PhantomData;

use composable_stream::ComposableStreamBuilder;

use crate::CreateBackupError;
use crate::config::CompressionConfig;

pub(crate) enum CompressionWriter<'a, W: Write> {
    #[cfg(feature = "zstd")]
    Zstd(zstd::Encoder<'a, W>),

    Off {
        writer: W,
        _marker: PhantomData<&'a ()>,
    },
}

pub(crate) fn compress<'a, W: Write>(
    config: &CompressionConfig,
) -> ComposableStreamBuilder<impl FnOnce(W) -> Result<CompressionWriter<'a, W>, CreateBackupError>>
{
    ComposableStreamBuilder {
        make: move |writer: W| match config {
            #[cfg(feature = "zstd")]
            CompressionConfig::Zstd { config } => {
                match zstd::Encoder::new(writer, config.compression_level) {
                    Ok(encoder) => Ok(CompressionWriter::Zstd(encoder)),
                    Err(err) => Err(CreateBackupError::CannotCompress(
                        anyhow::Error::from(err).context("Could not build zstd encoder"),
                    )),
                }
            }

            CompressionConfig::Off => Ok(CompressionWriter::Off {
                writer,
                _marker: PhantomData,
            }),
        },
    }
}

impl<'a, W: Write> Write for CompressionWriter<'a, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(feature = "zstd")]
            Self::Zstd(encoder) => encoder.write(buf),

            Self::Off { writer, .. } => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(feature = "zstd")]
            Self::Zstd(encoder) => encoder.flush(),

            Self::Off { writer, .. } => writer.flush(),
        }
    }
}

impl<'a, W: Write> CompressionWriter<'a, W> {
    pub fn finalize(self) -> Result<W, anyhow::Error> {
        match self {
            #[cfg(feature = "zstd")]
            Self::Zstd(encoder) => encoder.finish().map_err(anyhow::Error::new),

            Self::Off { writer, .. } => Ok(writer),
        }
    }
}
