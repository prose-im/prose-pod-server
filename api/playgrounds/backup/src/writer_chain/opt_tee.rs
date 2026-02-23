// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::writer_chain::{either::Either, tee::TeeWriter};

use super::WriterChainBuilder;

impl<M, F> WriterChainBuilder<M, F> {
    /// NOTE: Accepts a mutable reference to leave ownership to the called and
    ///   allow it to finalize the other writer manually.
    pub fn opt_tee<'a, InnerWriter, InnerWriter2, OuterWriter, Out, E>(
        self,
        other_writer: Option<&'a mut InnerWriter2>,
    ) -> WriterChainBuilder<
        impl FnOnce(InnerWriter) -> Result<OuterWriter, E>,
        impl FnOnce(OuterWriter) -> Result<Out, E>,
    >
    where
        M: FnOnce(
            Either<TeeWriter<InnerWriter, &'a mut InnerWriter2>, InnerWriter>,
        ) -> Result<OuterWriter, E>,
        F: FnOnce(OuterWriter) -> Result<Out, E>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer| {
                make(match other_writer {
                    Some(other_writer) => Either::A(TeeWriter::new(writer, other_writer)),
                    None => Either::B(writer),
                })
            },

            finalize: move |writer: OuterWriter| {
                let writer = finalize(writer)?;

                Ok(writer)
            },
        }
    }
}
