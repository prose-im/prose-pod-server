// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use super::WriterChainBuilder;
use super::either::Either;

#[inline]
pub fn eventually<A, B, MakeErr, FinalizeErr, T, M1, F1>(
    cond: Option<T>,
    other_builder: impl FnOnce(T) -> WriterChainBuilder<M1, F1>,
) -> WriterChainBuilder<
    impl FnOnce(A) -> Result<Either<B, A>, MakeErr>,
    impl FnOnce(Either<B, A>) -> Result<A, FinalizeErr>,
>
where
    M1: FnOnce(A) -> Result<B, MakeErr>,
    F1: FnOnce(B) -> Result<A, FinalizeErr>,
{
    let (make_b_opt, finalize_b_opt) = match cond {
        Some(t) => {
            let other: WriterChainBuilder<M1, F1> = other_builder(t);
            (Some(other.make), Some(other.finalize))
        }
        None => (None, None),
    };

    WriterChainBuilder {
        make: move |a: A| match make_b_opt {
            Some(make_b) => {
                let b: B = make_b(a)?;
                Ok(Either::A(b))
            }
            None => Ok(Either::B(a)),
        },

        finalize: move |e: Either<B, A>| match e {
            Either::A(b) => match finalize_b_opt {
                Some(finalize_b) => finalize_b(b),
                None => unreachable!(),
            },
            Either::B(a) => Ok(a),
        },
    }
}

#[inline]
pub fn optionally<A, B, Out, MakeErr, FinalizeErr, T, M1, F1>(
    cond: Option<T>,
    other_builder: impl FnOnce(T) -> WriterChainBuilder<M1, F1>,
) -> WriterChainBuilder<
    impl FnOnce(A) -> Result<Option<B>, MakeErr>,
    impl FnOnce(Option<B>) -> Result<Option<Out>, FinalizeErr>,
>
where
    M1: FnOnce(A) -> Result<B, MakeErr>,
    F1: FnOnce(B) -> Result<Out, FinalizeErr>,
{
    let (make_b_opt, finalize_b_opt) = match cond {
        Some(t) => {
            let other: WriterChainBuilder<M1, F1> = other_builder(t);
            (Some(other.make), Some(other.finalize))
        }
        None => (None, None),
    };

    WriterChainBuilder {
        make: move |a: A| match make_b_opt {
            Some(make_b) => {
                let b: B = make_b(a)?;
                Ok(Some(b))
            }
            None => Ok(None),
        },

        finalize: move |opt: Option<B>| match opt {
            Some(b) => match finalize_b_opt {
                Some(finalize_b) => finalize_b(b).map(Some),
                None => unreachable!(),
            },
            None => Ok(None),
        },
    }
}
