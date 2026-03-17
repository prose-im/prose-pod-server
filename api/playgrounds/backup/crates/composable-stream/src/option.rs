// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::{ComposableStreamBuilder, Either};

/// Add a layer, or don’t. Either way, you’ll get the same output type.
///
/// ```text
///              ◯
///              │ Condition
///              ◇──────┐
///         Some │      │ None
///          ┌───┴───┐  │
///          │ Other │  │
///          └───┬───┘  │
///              ◇──────┘
///              │
///              ◯ Output
/// ```
///
/// See also: [`optionally`].
#[inline]
pub fn eventually<T, A, B, MakeErr, FinalizeErr, M1, F1>(
    cond: Option<T>,
    other_builder: impl FnOnce(T) -> ComposableStreamBuilder<M1, F1>,
) -> ComposableStreamBuilder<
    impl FnOnce(A) -> Result<Either<B, A>, MakeErr>,
    impl FnOnce(Either<B, A>) -> Result<A, FinalizeErr>,
>
where
    M1: FnOnce(A) -> Result<B, MakeErr>,
    F1: FnOnce(B) -> Result<A, FinalizeErr>,
{
    let (make_b_opt, finalize_b_opt) = match cond {
        Some(t) => {
            let other: ComposableStreamBuilder<M1, F1> = other_builder(t);
            (Some(other.make), Some(other.finalize))
        }
        None => (None, None),
    };

    ComposableStreamBuilder {
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
                // NOTE: It’d be tempting to return two different
                //   `ComposableStreamBuilder` depending on `cond` to
                //   avoid this `unreachable`, but because we return
                //   anonymous closures Rust doesn’t allow it.
                // TODO: Add benchmarks, then see if we can change the
                //   whole API to use boxed closures.
                None => unreachable!(),
            },
            Either::B(a) => Ok(a),
        },
    }
}

/// Add a layer, or don’t. Output will be optional.
///
/// ```text
///              ◯
///              │ Condition
///              ◇──────┐
///         Some │      │ None
///          ┌───┴───┐  │
///          │ Other │  ◯ None
///          └───┬───┘
///              ◯ Some(Output)
/// ```
///
/// See also: [`eventually`].
#[inline]
pub fn optionally<T, A, B, Out, MakeErr, FinalizeErr, M1, F1>(
    cond: Option<T>,
    other_builder: impl FnOnce(T) -> ComposableStreamBuilder<M1, F1>,
) -> ComposableStreamBuilder<
    impl FnOnce(A) -> Result<Option<B>, MakeErr>,
    impl FnOnce(Option<B>) -> Result<Option<Out>, FinalizeErr>,
>
where
    M1: FnOnce(A) -> Result<B, MakeErr>,
    F1: FnOnce(B) -> Result<Out, FinalizeErr>,
{
    let (make_b_opt, finalize_b_opt) = match cond {
        Some(t) => {
            let other: ComposableStreamBuilder<M1, F1> = other_builder(t);
            (Some(other.make), Some(other.finalize))
        }
        None => (None, None),
    };

    ComposableStreamBuilder {
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
                // NOTE: It’d be tempting to return two different
                //   `ComposableStreamBuilder` depending on `cond` to
                //   avoid this `unreachable`, but because we return
                //   anonymous closures Rust doesn’t allow it.
                // TODO: Add benchmarks, then see if we can change the
                //   whole API to use boxed closures.
                None => unreachable!(),
            },
            None => Ok(None),
        },
    }
}
