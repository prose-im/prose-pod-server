// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use super::WriterChainBuilder;
use super::tee::TeeWriter;

impl<M, F> WriterChainBuilder<M, F> {
    #[inline]
    pub fn opt_tee_into<A1, B1, Out1, MakeErr1, FinalizeErr1, T, B2>(
        self,
        cond: Option<T>,
        other_builder: impl FnOnce(T) -> B2,
    ) -> WriterChainBuilder<
        impl FnOnce(A1) -> Result<TeeWriter<B1, Option<B2>>, MakeErr1>,
        impl FnOnce(TeeWriter<B1, Option<B2>>) -> (Result<Out1, FinalizeErr1>, Option<B2>),
    >
    where
        M: FnOnce(A1) -> Result<B1, MakeErr1>,
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

    #[inline]
    pub fn opt_tee<A1, B1, Res1, T, A2, B2, Out2, MakeErr, FinalizeErr, M2, F2>(
        self,
        cond: Option<T>,
        other_builder: impl FnOnce(T) -> WriterChainBuilder<M2, F2>,
        writer: A2,
    ) -> WriterChainBuilder<
        impl FnOnce(A1) -> Result<TeeWriter<B1, Option<B2>>, MakeErr>,
        impl FnOnce(TeeWriter<B1, Option<B2>>) -> (Res1, Option<Result<Out2, FinalizeErr>>),
    >
    where
        M: FnOnce(A1) -> Result<B1, MakeErr>,
        F: FnOnce(B1) -> Res1,
        M2: FnOnce(A2) -> Result<B2, MakeErr>,
        F2: FnOnce(B2) -> Result<Out2, FinalizeErr>,
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
                let res1: Res1 = finalize(tee.w1);

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
