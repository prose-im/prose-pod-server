// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! A library providing APIs to compose `Write`rs and `Read`ers in topological
//! order.
//!
//! Usually, layers streams in reverse topological order, meaning they must
//! create the “last” stream first, then build on it to finaly achieve the
//! desired shape. Code becomes harder to reason about, but also it’s likely
//! for someone to forget “finalizing” a stream in the chain, resulting in
//! an incorrect/corrupted output.
//!
//! With this library, streams can be described as a logical sequence of steps,
//! then finalized to get their output.
//!
//! See [`ComposableStreamBuilder::then`] for an example.

mod either;
mod option;
mod tee;

pub use self::ComposableStreamBuilder as Builder;
pub use self::either::*;
pub use self::option::*;
pub use self::tee::*;

// MARK: - Builder

pub struct ComposableStreamBuilder<Make, Finalize> {
    pub make: Make,
    pub finalize: Finalize,
}

#[inline]
pub fn builder<S, MakeErr, FinalizeErr>() -> ComposableStreamBuilder<
    // NOTE: We need `S -> S` here as this layer will be
    //   the outer-most layer when building the final stream.
    impl FnOnce(S) -> Result<S, MakeErr>,
    impl FnOnce(S) -> Result<S, FinalizeErr>,
> {
    ComposableStreamBuilder {
        make: move |stream: S| Ok(stream),
        finalize: move |stream: S| Ok(stream),
    }
}

impl<M, F> ComposableStreamBuilder<M, F> {
    /// Given a `W -> Bar<Baz<W>>` builder, augment a `W -> Foo<W>` so it
    /// becomes `W -> Foo<Bar<Baz<W>>>`. For example:
    ///
    /// ```no_run
    /// use anyhow::Context as _;
    /// use std::io::Write;
    ///
    /// # fn example() -> Result<(), anyhow::Error> {
    /// fn archive<W: Write>(
    ///     path: impl AsRef<std::path::Path>,
    /// ) -> composable_stream::Builder<
    ///     impl FnOnce(W) -> Result<tar::Builder<W>, anyhow::Error>,
    ///     impl FnOnce(tar::Builder<W>) -> Result<W, anyhow::Error>,
    /// > {
    ///     composable_stream::Builder {
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
    /// fn compress<W: Write>(level: i32) -> composable_stream::Builder<
    ///     impl FnOnce(W) -> Result<zstd::Encoder<'static, W>, anyhow::Error>,
    ///     impl FnOnce(zstd::Encoder<'static, W>) -> Result<W, anyhow::Error>,
    /// > {
    ///     composable_stream::Builder {
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
    /// let writer = composable_stream::builder() // W₀ -> W₀
    ///     .then(archive("foo/bar")) // W₁ -> tar::Builder<W₁>
    ///     .then(compress(3)) // W₂ -> zstd::Encoder<tar::Builder<W₂>>
    ///     .build(out)?; // zstd::Encoder<tar::Builder<File>>
    ///
    /// let _file: std::fs::File = writer.finalize()?;
    /// #     Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn then<A, B, C, Out, MakeErr, FinalizeErr>(
        self,
        other: ComposableStreamBuilder<
            impl FnOnce(A) -> Result<B, MakeErr>,
            impl FnOnce(B) -> Result<Out, FinalizeErr>,
        >,
    ) -> ComposableStreamBuilder<
        impl FnOnce(A) -> Result<C, MakeErr>,
        impl FnOnce(C) -> Result<Out, FinalizeErr>,
    >
    where
        M: FnOnce(B) -> Result<C, MakeErr>,
        F: FnOnce(C) -> Result<B, FinalizeErr>,
    {
        let Self { make, finalize, .. } = self;

        ComposableStreamBuilder {
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

impl<M, F> ComposableStreamBuilder<M, F> {
    #[inline]
    pub fn build<A, C, Out, Err>(self, stream: A) -> Result<FinalizableStream<C, F>, Err>
    where
        M: FnOnce(A) -> Result<C, Err>,
        F: FnOnce(C) -> Out,
    {
        let Self { make, finalize, .. } = self;

        make(stream).map(move |stream| FinalizableStream { stream, finalize })
    }
}

pub struct FinalizableStream<S, F> {
    pub stream: S,
    finalize: F,
}

impl<S, F> FinalizableStream<S, F> {
    #[inline]
    pub fn finalize<Out>(self) -> Out
    where
        F: FnOnce(S) -> Out,
    {
        (self.finalize)(self.stream)
    }
}
