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
    pub fn tee<A1, B1, A2, B2, Out1, Out2, MakeErr, FinalizeErr1, FinalizeErr2>(
        self,
        other: ComposableStreamBuilder<
            impl FnOnce(A2) -> Result<B2, MakeErr>,
            impl FnOnce(B2) -> Result<Out2, FinalizeErr2>,
        >,
        writer: A2,
    ) -> ComposableStreamBuilder<
        impl FnOnce(A1) -> Result<TeeStream<B1, B2>, MakeErr>,
        impl FnOnce(TeeStream<B1, B2>) -> (Result<Out1, FinalizeErr1>, Result<Out2, FinalizeErr2>),
    >
    where
        M1: FnOnce(A1) -> Result<B1, MakeErr>,
        F1: FnOnce(B1) -> Result<Out1, FinalizeErr1>,
    {
        let Self { make, finalize, .. } = self;

        ComposableStreamBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;
                let b2: B2 = (other.make)(writer)?;
                Ok(TeeStream::new(b1, b2))
            },

            finalize: move |tee: TeeStream<B1, B2>| {
                let res1: Result<Out1, FinalizeErr1> = finalize(tee.w1);
                let res2: Result<Out2, FinalizeErr2> = (other.finalize)(tee.w2);
                (res1, res2)
            },
        }
    }

    /// Same as [`tee`], but forks into the passed stream instead of using
    /// a stream builder.
    #[inline]
    pub fn tee_into<A1, B1, B2, Out1, MakeErr, FinalizeErr>(
        self,
        b2: B2,
    ) -> ComposableStreamBuilder<
        impl FnOnce(A1) -> Result<TeeStream<B1, B2>, MakeErr>,
        impl FnOnce(TeeStream<B1, B2>) -> (Result<Out1, FinalizeErr>, B2),
    >
    where
        M1: FnOnce(A1) -> Result<B1, MakeErr>,
        F1: FnOnce(B1) -> Result<Out1, FinalizeErr>,
    {
        let Self { make, finalize, .. } = self;

        ComposableStreamBuilder {
            make: move |a1: A1| {
                let b1: B1 = make(a1)?;
                Ok(TeeStream::new(b1, b2))
            },

            finalize: move |tee: TeeStream<B1, B2>| {
                let res1: Result<Out1, FinalizeErr> = finalize(tee.w1);
                (res1, tee.w2)
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
