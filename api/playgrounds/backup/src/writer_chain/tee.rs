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
    pub fn tee<A1, A2, B1, B2, Out1, Out2, Err, Err1, Err2>(
        self,
        other: WriterChainBuilder<
            impl FnOnce(A2) -> Result<B2, Err>,
            impl FnOnce(B2) -> Result<Out2, Err2>,
        >,
        writer: A2,
    ) -> WriterChainBuilder<
        impl FnOnce(A1) -> Result<TeeWriter<B1, B2>, Err>,
        impl FnOnce(TeeWriter<B1, B2>) -> (Result<Out1, Err1>, Result<Out2, Err2>),
    >
    where
        M: FnOnce(A1) -> Result<B1, Err>,
        F: FnOnce(B1) -> Result<Out1, Err1>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;
                let b2: B2 = (other.make)(writer)?;
                Ok(TeeWriter::new(b1, b2))
            },

            finalize: move |tee: TeeWriter<B1, B2>| {
                let res1: Result<Out1, Err1> = finalize(tee.w1);
                let res2: Result<Out2, Err2> = (other.finalize)(tee.w2);
                (res1, res2)
            },
        }
    }

    pub fn tee_into<A1, B1, Out1, MakeErr, FinalizeErr, B2>(
        self,
        b2: B2,
    ) -> WriterChainBuilder<
        impl FnOnce(A1) -> Result<TeeWriter<B1, B2>, MakeErr>,
        impl FnOnce(TeeWriter<B1, B2>) -> (Result<Out1, FinalizeErr>, B2),
    >
    where
        M: FnOnce(A1) -> Result<B1, MakeErr>,
        F: FnOnce(B1) -> Result<Out1, FinalizeErr>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;
                Ok(TeeWriter::new(b1, b2))
            },

            finalize: move |tee: TeeWriter<B1, B2>| {
                let res1: Result<Out1, FinalizeErr> = finalize(tee.w1);
                (res1, tee.w2)
            },
        }
    }
}

pub fn tee<B1, B2, MakeErr, FinalizeErr>(
    b2: B2,
) -> WriterChainBuilder<
    impl FnOnce(B1) -> Result<TeeWriter<B1, B2>, MakeErr>,
    impl FnOnce(TeeWriter<B1, B2>) -> Result<(B1, B2), FinalizeErr>,
> {
    WriterChainBuilder {
        make: move |b1: B1| Ok(TeeWriter::new(b1, b2)),
        finalize: move |tee: TeeWriter<B1, B2>| Ok((tee.w1, tee.w2)),
    }
}
