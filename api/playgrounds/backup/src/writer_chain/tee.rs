// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::{self, Write};

use super::WriterChainBuilder;

/// A [`Write`]r that duplicates input into two underlying `Write`rs.
///
/// See [“tee (command)” on Wikipedia][tee] for more information about the
/// naming and use cases.
///
/// [tee]: https://en.wikipedia.org/wiki/Tee_(command)
pub struct TeeWriter<W1, W2> {
    pub w1: W1,
    pub w2: W2,
}

impl<W1, W2> TeeWriter<W1, W2> {
    pub fn new(w1: W1, w2: W2) -> Self {
        Self { w1, w2 }
    }
}

impl<W1: Write, W2: Write> Write for TeeWriter<W1, W2> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Write to first writer
        let n = self.w1.write(buf)?;

        // Write the same amount to the second writer
        // If this fails, return that error
        self.w2.write_all(&buf[..n])?;

        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.w1.flush()?;
        self.w2.flush()?;
        Ok(())
    }
}

impl<M, F> WriterChainBuilder<M, F> {
    /// NOTE: Accepts a mutable reference to leave ownership to the called and
    ///   allow it to finalize the other writer manually.
    pub fn tee<InnerWriter, InnerWriter2, OuterWriter, Out, E>(
        self,
        other_writer: InnerWriter2,
    ) -> WriterChainBuilder<
        impl FnOnce(InnerWriter) -> Result<OuterWriter, E>,
        impl FnOnce(OuterWriter) -> Result<Out, E>,
    >
    where
        M: FnOnce(TeeWriter<InnerWriter, InnerWriter2>) -> Result<OuterWriter, E>,
        F: FnOnce(OuterWriter) -> Result<Out, E>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer| make(TeeWriter::new(writer, other_writer)),

            finalize: move |writer: OuterWriter| {
                let writer = finalize(writer)?;

                Ok(writer)
            },
        }
    }
}
