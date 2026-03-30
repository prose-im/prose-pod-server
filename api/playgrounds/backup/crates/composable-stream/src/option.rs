// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::{ComposableStreamBuilder, Either};

pub enum OptionalStream<S> {
    None,
    Some(S),
}

impl<S> OptionalStream<S> {
    #[inline]
    pub fn new(mut stream: S, cond: impl FnOnce(&mut S) -> bool) -> Self {
        if cond(&mut stream) {
            Self::Some(stream)
        } else {
            Self::None
        }
    }

    #[inline]
    pub const fn is_some(&self) -> bool {
        matches!(*self, Self::Some(_))
    }

    #[inline]
    pub const fn is_none(&self) -> bool {
        !self.is_some()
    }

    #[inline]
    pub fn map<T, F>(option: Option<T>, f: F) -> Self
    where
        F: FnOnce(T) -> S,
    {
        match option {
            Some(t) => Self::Some(f(t)),
            None => Self::None,
        }
    }

    pub const fn take(&mut self) -> Self {
        // FIXME(const-hack) replace `mem::replace` by `mem::take` when the latter is const ready
        std::mem::replace(self, Self::None)
    }
}

impl<T> std::io::Write for OptionalStream<T>
where
    T: std::io::Write,
{
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::None => Ok(buf.len()),
            Self::Some(writer) => writer.write(buf),
        }
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::None => Ok(()),
            Self::Some(writer) => writer.flush(),
        }
    }
}

impl<T> std::io::Read for OptionalStream<T>
where
    T: std::io::Read,
{
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::None => Ok(0),
            Self::Some(reader) => reader.read(buf),
        }
    }
}

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
) -> ComposableStreamBuilder<impl FnOnce(A) -> Result<OptionalStream<B>, Err>>
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
                Ok(OptionalStream::Some(b))
            }
            None => Ok(OptionalStream::None),
        },
    }
}
