// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::writer_chain::either::Either;

pub mod either;
pub mod opt_tee;
mod tee;

pub use self::WriterChainBuilder as Builder;
pub use self::tee::*;

pub struct WriterChainBuilder<Make, Finalize> {
    pub make: Make,
    pub finalize: Finalize,
}

pub fn builder<W, E>() -> WriterChainBuilder<
    // NOTE: We need `W -> W` here as this layer will be
    //   the outer-most layer when building the final writer.
    impl FnOnce(W) -> Result<W, E>,
    impl FnOnce(W) -> Result<W, E>,
> {
    WriterChainBuilder {
        make: move |writer: W| Ok(writer),
        finalize: move |writer: W| Ok(writer),
    }
}

impl<M, F> WriterChainBuilder<M, F> {
    /// Given a `W -> Bar<Baz<W>>` builder, augment a `W -> Foo<W>` so it
    /// becomes `W -> Foo<Bar<Baz<W>>>`. For example:
    ///
    /// ```no_run
    /// use anyhow::Context as _;
    /// use prose_backup::writer_chain;
    /// use std::io::Write;
    ///
    /// # fn example() -> Result<(), anyhow::Error> {
    /// fn archive<W: Write>(
    ///     path: impl AsRef<std::path::Path>,
    /// ) -> writer_chain::Builder<
    ///     impl FnOnce(W) -> Result<tar::Builder<W>, anyhow::Error>,
    ///     impl FnOnce(tar::Builder<W>) -> Result<W, anyhow::Error>,
    /// > {
    ///     writer_chain::Builder {
    ///         make: move |writer: W| {
    ///             let mut builder: tar::Builder<_> = tar::Builder::new(writer);
    ///             builder.append_path(path)?;
    ///             Ok(builder)
    ///         },
    ///         finalize: move |writer: tar::Builder<W>| {
    ///             writer.into_inner().context("Failed archiving")
    ///         },
    ///     }
    /// }
    ///
    /// fn compress<W: Write>(level: i32) -> writer_chain::Builder<
    ///     impl FnOnce(W) -> Result<zstd::Encoder<'static, W>, anyhow::Error>,
    ///     impl FnOnce(zstd::Encoder<'static, W>) -> Result<W, anyhow::Error>,
    /// > {
    ///     writer_chain::Builder {
    ///         make: move |writer: W| {
    ///             zstd::Encoder::new(writer, level)
    ///                 .context("Failed creating zstd encoder")
    ///         },
    ///         finalize: move |writer: zstd::Encoder<'static, W>| {
    ///             writer.finish().context("Failed compressing with zstd")
    ///         },
    ///     }
    /// }
    ///
    /// let out = std::fs::File::open("/dev/null")?; // File
    ///
    /// let (writer, finalize) = writer_chain::builder() // W -> W
    ///     .then(archive("foo/bar")) // W -> tar::Builder<W>
    ///     .then(compress(3)) // W -> zstd::Encoder<tar::Builder<W>>
    ///     .build(out)?; // zstd::Encoder<tar::Builder<File>>
    ///
    /// finalize(writer)?;
    /// #     Ok(())
    /// # }
    /// ```
    pub fn then<A, B, C, Out, Err>(
        self,
        other: WriterChainBuilder<
            impl FnOnce(A) -> Result<B, Err>,
            impl FnOnce(B) -> Result<Out, Err>,
        >,
    ) -> WriterChainBuilder<impl FnOnce(A) -> Result<C, Err>, impl FnOnce(C) -> Result<Out, Err>>
    where
        M: FnOnce(B) -> Result<C, Err>,
        F: FnOnce(C) -> Result<B, Err>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |a: A| {
                let b: B = (other.make)(a)?;
                make(b)
            },

            finalize: move |c: C| {
                let b: B = finalize(c)?;
                (other.finalize)(b)
            },
        }
    }

    #[must_use]
    pub fn build<InnermostWriter, OuterWriter, Out, Err>(
        self,
        writer: InnermostWriter,
    ) -> Result<FinalizableWriter<OuterWriter, F>, Err>
    where
        InnermostWriter: std::io::Write,
        M: FnOnce(InnermostWriter) -> Result<OuterWriter, Err>,
        F: FnOnce(OuterWriter) -> Out,
    {
        let Self { make, finalize, .. } = self;

        make(writer).map(move |writer| FinalizableWriter { writer, finalize })
    }
}

pub fn eventually<
    A,
    B,
    MakeErr,
    FinalizeErr,
    T,
    M1: FnOnce(A) -> Result<B, MakeErr>,
    F1: FnOnce(B) -> Result<A, FinalizeErr>,
>(
    cond: Option<T>,
    other_builder: impl FnOnce(T) -> WriterChainBuilder<M1, F1>,
) -> WriterChainBuilder<
    impl FnOnce(A) -> Result<Either<B, A>, MakeErr>,
    impl FnOnce(Either<B, A>) -> Result<A, FinalizeErr>,
> {
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

pub fn optionally<
    A,
    B,
    Out,
    MakeErr,
    FinalizeErr,
    T,
    M1: FnOnce(A) -> Result<B, MakeErr>,
    F1: FnOnce(B) -> Result<Out, FinalizeErr>,
>(
    cond: Option<T>,
    other_builder: impl FnOnce(T) -> WriterChainBuilder<M1, F1>,
) -> WriterChainBuilder<
    impl FnOnce(A) -> Result<Option<B>, MakeErr>,
    impl FnOnce(Option<B>) -> Result<Option<Out>, FinalizeErr>,
> {
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

// MARK: - Build

pub struct FinalizableWriter<W, Finalize> {
    pub writer: W,
    finalize: Finalize,
}

impl<W, Finalize> FinalizableWriter<W, Finalize> {
    pub fn finalize<Out>(self) -> Out
    where
        Finalize: FnOnce(W) -> Out,
    {
        (self.finalize)(self.writer)
    }
}
