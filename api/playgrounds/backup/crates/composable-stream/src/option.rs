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
pub fn eventually<T, A, B, Err, M1>(
    cond: Option<T>,
    other_builder: impl FnOnce(T) -> ComposableStreamBuilder<M1>,
) -> ComposableStreamBuilder<impl FnOnce(A) -> Result<Either<B, A>, Err>>
where
    M1: FnOnce(A) -> Result<B, Err>,
{
    let make_b_opt = match cond {
        Some(t) => {
            let other: ComposableStreamBuilder<M1> = other_builder(t);
            Some(other.make)
        }
        None => None,
    };

    ComposableStreamBuilder {
        make: move |a: A| match make_b_opt {
            Some(make_b) => {
                let b: B = make_b(a)?;
                Ok(Either::A(b))
            }
            None => Ok(Either::B(a)),
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
pub fn optionally<T, A, B, Err, M1>(
    cond: Option<T>,
    other_builder: impl FnOnce(T) -> ComposableStreamBuilder<M1>,
) -> ComposableStreamBuilder<impl FnOnce(A) -> Result<Option<B>, Err>>
where
    M1: FnOnce(A) -> Result<B, Err>,
{
    let make_b_opt = match cond {
        Some(t) => {
            let other: ComposableStreamBuilder<M1> = other_builder(t);
            Some(other.make)
        }
        None => None,
    };

    ComposableStreamBuilder {
        make: move |a: A| match make_b_opt {
            Some(make_b) => {
                let b: B = make_b(a)?;
                Ok(Some(b))
            }
            None => Ok(None),
        },
    }
}
