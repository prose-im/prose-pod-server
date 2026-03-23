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
pub fn tee<B1, B2, MakeErr, FinalizeErr>(
    b2: B2,
) -> ComposableStreamBuilder<
    impl FnOnce(B1) -> Result<TeeStream<B1, B2>, MakeErr>,
    impl FnOnce(TeeStream<B1, B2>) -> Result<(B1, B2), FinalizeErr>,
> {
    ComposableStreamBuilder {
        make: move |b1: B1| Ok(TeeStream::new(b1, b2)),
        finalize: move |tee: TeeStream<B1, B2>| Ok((tee.w1, tee.w2)),
    }
}

// MARK: Composable tee

impl<M1, F1> ComposableStreamBuilder<M1, F1> {
    /// Fork the stream into two. Output will be a pair of `Result`s.
    ///
    /// See [“tee (command)” on Wikipedia][tee-wiki] for more information about
    /// the naming and use cases.
    ///
    /// [tee-wiki]: https://en.wikipedia.org/wiki/Tee_(command)
    #[inline]
    pub fn tee<B1, A2, B2, C, Res2, MakeErr, FinalizeErr1>(
        self,
        other: ComposableStreamBuilder<
            impl FnOnce(A2) -> Result<B2, MakeErr>,
            impl FnOnce(B2) -> Res2,
        >,
        a2: A2,
    ) -> ComposableStreamBuilder<
        impl FnOnce(B1) -> Result<C, MakeErr>,
        impl FnOnce(C) -> Result<(B1, Res2), FinalizeErr1>,
    >
    where
        M1: FnOnce(TeeStream<B1, B2>) -> Result<C, MakeErr>,
        F1: FnOnce(C) -> Result<TeeStream<B1, B2>, FinalizeErr1>,
    {
        let Self { make, finalize, .. } = self;

        ComposableStreamBuilder {
            make: move |b1: B1| {
                let b2: B2 = (other.make)(a2)?;
                let tee: TeeStream<B1, B2> = TeeStream::new(b1, b2);
                make(tee)
            },

            finalize: move |c: C| {
                let tee: TeeStream<B1, B2> = finalize(c)?;
                let res2: Res2 = (other.finalize)(tee.w2);
                Ok((tee.w1, res2))
            },
        }
    }

    /// Same as [`tee`], but forks into the passed stream instead of using
    /// a stream builder.
    #[inline]
    pub fn tee_into<B1, B2, C, MakeErr, FinalizeErr>(
        self,
        b2: B2,
    ) -> ComposableStreamBuilder<
        impl FnOnce(B1) -> Result<C, MakeErr>,
        impl FnOnce(C) -> Result<(B1, B2), FinalizeErr>,
    >
    where
        M1: FnOnce(TeeStream<B1, B2>) -> Result<C, MakeErr>,
        F1: FnOnce(C) -> Result<TeeStream<B1, B2>, FinalizeErr>,
    {
        let Self { make, finalize, .. } = self;

        ComposableStreamBuilder {
            make: move |b1: B1| {
                let tee: TeeStream<B1, B2> = TeeStream::new(b1, b2);
                make(tee)
            },

            finalize: move |c: C| {
                let tee = finalize(c)?;
                Ok((tee.w1, tee.w2))
            },
        }
    }
}

// MARK: Composable optional tee

impl<M1, F1> ComposableStreamBuilder<M1, F1> {
    /// Fork, or don’t. Second output will be optional.
    #[inline]
    pub fn opt_tee<T, A1, B1, A2, B2, Res1, Res2, MakeErr, M2, F2>(
        self,
        cond: Option<T>,
        other_builder: impl FnOnce(T) -> ComposableStreamBuilder<M2, F2>,
        stream: A2,
    ) -> ComposableStreamBuilder<
        impl FnOnce(A1) -> Result<Tee<B1, Option<B2>>, MakeErr>,
        impl FnOnce(Tee<B1, Option<B2>>) -> (Res1, Option<Res2>),
    >
    where
        M1: FnOnce(A1) -> Result<B1, MakeErr>,
        F1: FnOnce(B1) -> Res1,
        M2: FnOnce(A2) -> Result<B2, MakeErr>,
        F2: FnOnce(B2) -> Res2,
    {
        let Self { make, finalize, .. } = self;

        let (make_b2_opt, finalize_b2_opt) = match cond {
            Some(t) => {
                let other: ComposableStreamBuilder<M2, F2> = other_builder(t);
                (Some(other.make), Some(other.finalize))
            }
            None => (None, None),
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

            finalize: move |tee: Tee<B1, Option<B2>>| {
                let res1: Res1 = finalize(tee.w1);

                let res2_opt: Option<Res2> = match tee.w2 {
                    Some(b2) => match finalize_b2_opt {
                        Some(finalize_b2) => Some(finalize_b2(b2)),
                        // NOTE: It’d be tempting to return two different
                        //   `ComposableStreamBuilder` depending on `cond` to
                        //   avoid this `unreachable`, but because we return
                        //   anonymous closures Rust doesn’t allow it.
                        // TODO: Add benchmarks, then see if we can change the
                        //   whole API to use boxed closures.
                        None => unreachable!(),
                    },
                    None => None,
                };

                (res1, res2_opt)
            },
        }
    }

    /// Same as [`opt_tee`], but forks into the passed stream instead of using
    /// a stream builder.
    #[inline]
    pub fn opt_tee_into<T, A1, B1, B2, Res1, MakeErr>(
        self,
        cond: Option<T>,
        other_builder: impl FnOnce(T) -> B2,
    ) -> ComposableStreamBuilder<
        impl FnOnce(A1) -> Result<Tee<B1, Option<B2>>, MakeErr>,
        impl FnOnce(Tee<B1, Option<B2>>) -> (Res1, Option<B2>),
    >
    where
        M1: FnOnce(A1) -> Result<B1, MakeErr>,
        F1: FnOnce(B1) -> Res1,
    {
        let Self { make, finalize, .. } = self;

        ComposableStreamBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;
                let b2_opt: Option<B2> = cond.map(other_builder);
                Ok(Tee::new(b1, b2_opt))
            },

            finalize: move |tee: Tee<B1, Option<B2>>| {
                let res1: Res1 = finalize(tee.w1);
                (res1, tee.w2)
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
        let out2 = times_two::<_, Infallible, Infallible>()
            .build(Vec::<u8>::new())
            .unwrap_or_else(unreachable);
        let tee = source(&[1, 2, 3])
            .then(add_one())
            .tee_into(out2.stream)
            .build(Vec::<u8>::new())?;

        let (res1, res2) = tee.finalize().unwrap_or_else(unreachable);
        let res2 = (out2.finalize)(res2).unwrap_or_else(unreachable);

        assert_eq!(res1.as_slice(), [2, 3, 4]);
        assert_eq!(res2.as_slice(), [4, 6, 8]);

        Ok(())
    }

    #[test]
    fn test_compose_source_tee() -> Result<(), std::io::Error> {
        let tee = source(&[1, 2, 3])
            .then(add_one())
            .tee(times_two(), Vec::<u8>::new())
            .build(Vec::<u8>::new())?;

        let (res1, res2) = tee.finalize().unwrap_or_else(unreachable);
        let res2 = res2.unwrap_or_else(unreachable);

        assert_eq!(res1.as_slice(), [2, 3, 4]);
        assert_eq!(res2.as_slice(), [4, 6, 8]);

        Ok(())
    }

    #[test]
    fn test_compose_builder_tee() -> Result<(), std::io::Error> {
        let mut tee: crate::FinalizableStream<Tee<Vec<u8>, Vec<u8>>, _> =
            crate::builder::<_, Infallible, Infallible>()
                .tee(
                    crate::builder::<_, Infallible, Infallible>(),
                    Vec::<u8>::new(),
                )
                .build(Vec::<u8>::new())
                .unwrap_or_else(unreachable);

        tee.stream.write_all(&[1, 2, 3])?;

        let (res1, res2) = tee.finalize().unwrap_or_else(unreachable);
        let res2 = res2.unwrap_or_else(unreachable);

        assert_eq!(res1.as_slice(), [1, 2, 3]);
        assert_eq!(res2.as_slice(), [1, 2, 3]);

        Ok(())
    }

    fn source<W: Write>(
        data: &[u8],
    ) -> ComposableStreamBuilder<
        impl FnOnce(W) -> Result<W, std::io::Error>,
        impl FnOnce(W) -> Result<W, Infallible>,
    > {
        ComposableStreamBuilder {
            make: move |mut writer: W| {
                writer.write_all(data)?;
                Ok(writer)
            },
            finalize: Ok,
        }
    }
}
