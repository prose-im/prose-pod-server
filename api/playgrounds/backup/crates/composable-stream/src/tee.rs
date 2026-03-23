// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::ComposableStreamBuilder;

pub use self::TeeStream as Tee;

/// A [`Write`]r that duplicates input into two underlying `Write`rs.
///
/// See [“tee (command)” on Wikipedia][tee-wiki] for more information about
/// the naming and use cases.
///
/// [tee-wiki]: https://en.wikipedia.org/wiki/Tee_(command)
pub struct TeeStream<W1, W2> {
    pub w1: W1,
    pub w2: W2,
}

impl<W1, W2> TeeStream<W1, W2> {
    #[inline]
    pub fn new(w1: W1, w2: W2) -> Self {
        Self { w1, w2 }
    }
}

#[inline]
pub fn tee<B1, B2, Err>(
    b2: B2,
) -> ComposableStreamBuilder<impl FnOnce(B1) -> Result<TeeStream<B1, B2>, Err>> {
    ComposableStreamBuilder {
        make: move |b1: B1| Ok(TeeStream::new(b1, b2)),
    }
}

// MARK: Composable tee

impl<M1> ComposableStreamBuilder<M1> {
    /// Fork the stream into two. Output will be a pair of `Result`s.
    ///
    /// See [“tee (command)” on Wikipedia][tee-wiki] for more information about
    /// the naming and use cases.
    ///
    /// [tee-wiki]: https://en.wikipedia.org/wiki/Tee_(command)
    #[inline]
    pub fn tee<B1, A2, B2, C, Err>(
        self,
        other: ComposableStreamBuilder<impl FnOnce(A2) -> Result<B2, Err>>,
        a2: A2,
    ) -> ComposableStreamBuilder<impl FnOnce(B1) -> Result<C, Err>>
    where
        M1: FnOnce(TeeStream<B1, B2>) -> Result<C, Err>,
    {
        let Self { make, .. } = self;

        ComposableStreamBuilder {
            make: move |b1: B1| {
                let b2: B2 = (other.make)(a2)?;
                let tee: TeeStream<B1, B2> = TeeStream::new(b1, b2);
                make(tee)
            },
        }
    }

    /// Same as [`tee`], but forks into the passed stream instead of using
    /// a stream builder.
    #[inline]
    pub fn tee_into<B1, B2, C, Err>(
        self,
        b2: B2,
    ) -> ComposableStreamBuilder<impl FnOnce(B1) -> Result<C, Err>>
    where
        M1: FnOnce(TeeStream<B1, B2>) -> Result<C, Err>,
    {
        let Self { make, .. } = self;

        ComposableStreamBuilder {
            make: move |b1: B1| {
                let tee: TeeStream<B1, B2> = TeeStream::new(b1, b2);
                make(tee)
            },
        }
    }
}

// MARK: Composable optional tee

impl<M1> ComposableStreamBuilder<M1> {
    /// Fork, or don’t. Second output will be optional.
    #[inline]
    pub fn opt_tee<T, A1, B1, A2, B2, Err, M2>(
        self,
        cond: Option<T>,
        other_builder: impl FnOnce(T) -> ComposableStreamBuilder<M2>,
        stream: A2,
    ) -> ComposableStreamBuilder<impl FnOnce(A1) -> Result<Tee<B1, Option<B2>>, Err>>
    where
        M1: FnOnce(A1) -> Result<B1, Err>,
        M2: FnOnce(A2) -> Result<B2, Err>,
    {
        let Self { make, .. } = self;

        let make_b2_opt = match cond {
            Some(t) => {
                let other: ComposableStreamBuilder<M2> = other_builder(t);
                Some(other.make)
            }
            None => None,
        };

        ComposableStreamBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;

                let b2_opt: Option<B2> = match make_b2_opt {
                    Some(make_b2) => {
                        let b2: B2 = make_b2(stream)?;
                        Some(b2)
                    }
                    None => None,
                };

                Ok(Tee::new(b1, b2_opt))
            },
        }
    }

    /// Same as [`opt_tee`], but forks into the passed stream instead of using
    /// a stream builder.
    #[inline]
    pub fn opt_tee_into<T, A1, B1, B2, Err>(
        self,
        cond: Option<T>,
        other_builder: impl FnOnce(T) -> B2,
    ) -> ComposableStreamBuilder<impl FnOnce(A1) -> Result<Tee<B1, Option<B2>>, Err>>
    where
        M1: FnOnce(A1) -> Result<B1, Err>,
    {
        let Self { make, .. } = self;

        ComposableStreamBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;
                let b2_opt: Option<B2> = cond.map(other_builder);
                Ok(Tee::new(b1, b2_opt))
            },
        }
    }
}

// MARK: - Boilerplate

impl<W1, W2> std::io::Write for TeeStream<W1, W2>
where
    W1: std::io::Write,
    W2: std::io::Write,
{
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.w1.write(buf)?;
        self.w2.write_all(&buf[..n])?;
        Ok(n)
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        self.w1.flush()?;
        self.w2.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, io::Write};

    use crate::tests::util::*;

    use super::*;

    #[test]
    fn test_tee_stream() -> Result<(), std::io::Error> {
        let mut tee = TeeStream::new(Vec::<u8>::new(), Vec::<u8>::new());

        tee.write_all(&[1, 2, 3])?;

        assert_eq!(tee.w1.as_slice(), [1, 2, 3]);
        assert_eq!(tee.w2.as_slice(), [1, 2, 3]);

        Ok(())
    }

    // #[test]
    // fn test_compose_source_then_tee() -> Result<(), std::io::Error> {
    //     let out2 = times_two().build(Vec::<u8>::new())?;
    //     let tee = source(&[1, 2, 3])
    //         .then(tee(out2.stream))
    //         .then(add_one())
    //         .build(Vec::<u8>::new())?;

    //     let (res1, res2) = tee.finalize().unwrap_or_else(unreachable);

    //     assert_eq!(res1.as_slice(), [2, 3, 4]);
    //     assert_eq!(res2.as_slice(), [2, 4, 6]);

    //     Ok(())
    // }

    #[test]
    fn test_compose_source_tee_into() -> Result<(), std::io::Error> {
        let out2 = times_two::<_, Infallible>()
            .build(Vec::<u8>::new())
            .unwrap_or_else(unreachable);
        let writer = source(&[1, 2, 3])
            .then(add_one())
            .tee_into(out2)
            .build(Vec::<u8>::new())?;

        let tee = writer.into_inner();

        let (res1, out2) = (tee.w1, tee.w2);
        let res2 = out2.into_inner();

        assert_eq!(res1.as_slice(), [2, 3, 4]);
        assert_eq!(res2.as_slice(), [4, 6, 8]);

        Ok(())
    }

    #[test]
    fn test_compose_source_tee() -> Result<(), std::io::Error> {
        let writer = source(&[1, 2, 3])
            .then(add_one())
            .tee(times_two(), Vec::<u8>::new())
            .build(Vec::<u8>::new())?;

        let tee = writer.into_inner();

        let (res1, out2) = (tee.w1, tee.w2);
        let res2 = out2.into_inner();

        assert_eq!(res1.as_slice(), [2, 3, 4]);
        assert_eq!(res2.as_slice(), [4, 6, 8]);

        Ok(())
    }

    #[test]
    fn test_compose_builder_tee() -> Result<(), std::io::Error> {
        let mut tee: Tee<Vec<u8>, Vec<u8>> = crate::builder()
            .tee(crate::builder(), Vec::<u8>::new())
            .build(Vec::<u8>::new())
            .unwrap_or_else(unreachable);

        tee.write_all(&[1, 2, 3])?;

        let (res1, res2) = (tee.w1, tee.w2);

        assert_eq!(res1.as_slice(), [1, 2, 3]);
        assert_eq!(res2.as_slice(), [1, 2, 3]);

        Ok(())
    }

    fn source<W: Write>(
        data: &[u8],
    ) -> ComposableStreamBuilder<impl FnOnce(W) -> Result<W, std::io::Error>> {
        ComposableStreamBuilder {
            make: move |mut writer: W| {
                writer.write_all(data)?;
                Ok(writer)
            },
        }
    }
}
