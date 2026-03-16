// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::writer_chain::{either::Either, tee::TeeWriter};

use super::WriterChainBuilder;

impl<M, F> WriterChainBuilder<M, F> {
    /// NOTE: Accepts a mutable reference to leave ownership to the called and
    ///   allow it to finalize the other writer manually.
    pub fn opt_tee_<'a, InnerWriter, InnerWriter2, OuterWriter, Out, E>(
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
            make: move |writer: InnerWriter| {
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

    pub fn opt_tee__<'a, InnerWriter, InnerWriter2, OuterWriter, Out, E>(
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

    pub fn opt_tee_into<A1, B1, B2, T, Out1, MakeErr, FinalizeErr1>(
        self,
        cond: Option<T>,
        other_builder: impl FnOnce(T) -> B2,
    ) -> WriterChainBuilder<
        impl FnOnce(A1) -> Result<TeeWriter<B1, Option<B2>>, MakeErr>,
        impl FnOnce(TeeWriter<B1, Option<B2>>) -> (Result<Out1, FinalizeErr1>, Option<B2>),
    >
    where
        M: FnOnce(A1) -> Result<B1, MakeErr>,
        F: FnOnce(B1) -> Result<Out1, FinalizeErr1>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;
                let b2_opt: Option<B2> = cond.map(other_builder);
                Ok(TeeWriter::new(b1, b2_opt))
            },

            finalize: move |tee: TeeWriter<B1, Option<B2>>| {
                let res1: Result<Out1, FinalizeErr1> = finalize(tee.w1);
                (res1, tee.w2)
            },
        }
    }

    pub fn opt_tee<A1, A2, B1, B2, T, Out1, Out2, MakeErr, FinalizeErr1, FinalizeErr2, M2, F2>(
        self,
        cond: Option<T>,
        other_builder: impl FnOnce(T) -> WriterChainBuilder<M2, F2>,
        writer: A2,
    ) -> WriterChainBuilder<
        impl FnOnce(A1) -> Result<TeeWriter<B1, Option<B2>>, MakeErr>,
        impl FnOnce(
            TeeWriter<B1, Option<B2>>,
        ) -> (
            Result<Out1, FinalizeErr1>,
            Option<Result<Out2, FinalizeErr2>>,
        ),
    >
    where
        M: FnOnce(A1) -> Result<B1, MakeErr>,
        F: FnOnce(B1) -> Result<Out1, FinalizeErr1>,
        M2: FnOnce(A2) -> Result<B2, MakeErr>,
        F2: FnOnce(B2) -> Result<Out2, FinalizeErr2>,
    {
        let Self { make, finalize, .. } = self;

        let (make_b2_opt, finalize_b2_opt) = match cond {
            Some(t) => {
                let other: WriterChainBuilder<M2, F2> = other_builder(t);
                (Some(other.make), Some(other.finalize))
            }
            None => (None, None),
        };

        WriterChainBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;

                let b2_opt = match make_b2_opt {
                    Some(make_b2) => {
                        let b2: B2 = make_b2(writer)?;
                        Ok(Some(b2))
                    }
                    None => Ok(None),
                }?;

                Ok(TeeWriter::new(b1, b2_opt))
            },

            finalize: move |tee: TeeWriter<B1, Option<B2>>| {
                let res1: Result<Out1, FinalizeErr1> = finalize(tee.w1);

                let res2_opt = match tee.w2 {
                    Some(b) => match finalize_b2_opt {
                        Some(finalize_b2) => Some(finalize_b2(b)),
                        None => unreachable!(),
                    },
                    None => None,
                };

                (res1, res2_opt)
            },
        }
    }
}
