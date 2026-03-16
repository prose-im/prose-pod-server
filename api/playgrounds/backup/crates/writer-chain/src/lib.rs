// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod either;
mod opt_tee;
mod option;
mod tee;

pub use self::WriterChainBuilder as Builder;
pub use self::either::*;
pub use self::option::*;
pub use self::tee::*;

// MARK: - Builder

pub struct WriterChainBuilder<Make, Finalize> {
    pub make: Make,
    pub finalize: Finalize,
}

#[inline(always)]
pub fn builder<W, MakeErr, FinalizeErr>() -> WriterChainBuilder<
    // NOTE: We need `W -> W` here as this layer will be
    //   the outer-most layer when building the final writer.
    impl FnOnce(W) -> Result<W, MakeErr>,
    impl FnOnce(W) -> Result<W, FinalizeErr>,
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
    /// let writer = writer_chain::builder() // W -> W
    ///     .then(archive("foo/bar")) // W -> tar::Builder<W>
    ///     .then(compress(3)) // W -> zstd::Encoder<tar::Builder<W>>
    ///     .build(out)?; // zstd::Encoder<tar::Builder<File>>
    ///
    /// writer.finalize()?;
    /// #     Ok(())
    /// # }
    /// ```
    #[inline(always)]
    pub fn then<A, B, C, Out, MakeErr, FinalizeErr>(
        self,
        other: WriterChainBuilder<
            impl FnOnce(A) -> Result<B, MakeErr>,
            impl FnOnce(B) -> Result<Out, FinalizeErr>,
        >,
    ) -> WriterChainBuilder<
        impl FnOnce(A) -> Result<C, MakeErr>,
        impl FnOnce(C) -> Result<Out, FinalizeErr>,
    >
    where
        M: FnOnce(B) -> Result<C, MakeErr>,
        F: FnOnce(C) -> Result<B, FinalizeErr>,
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
}

// MARK: - Build

impl<M, F> WriterChainBuilder<M, F> {
    #[must_use]
    #[inline(always)]
    pub fn build<A, C, Out, Err>(self, writer: A) -> Result<FinalizableWriter<C, F>, Err>
    where
        A: std::io::Write,
        M: FnOnce(A) -> Result<C, Err>,
        F: FnOnce(C) -> Out,
    {
        let Self { make, finalize, .. } = self;

        make(writer).map(move |writer| FinalizableWriter { writer, finalize })
    }
}

pub struct FinalizableWriter<W, Finalize> {
    pub writer: W,
    finalize: Finalize,
}

impl<W, Finalize> FinalizableWriter<W, Finalize> {
    #[inline(always)]
    pub fn finalize<Out>(self) -> Out
    where
        Finalize: FnOnce(W) -> Out,
    {
        (self.finalize)(self.writer)
    }
}
