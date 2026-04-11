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

pub struct ComposableStreamBuilder<Make> {
    pub make: Make,
}

#[inline]
pub fn builder<S, Err>() -> ComposableStreamBuilder<
    // NOTE: We need `S -> S` here as this layer will be
    //   the outer-most layer when building the final stream.
    impl FnOnce(S) -> Result<S, Err>,
> {
    ComposableStreamBuilder {
        make: move |stream: S| Ok(stream),
    }
}

impl<M> ComposableStreamBuilder<M> {
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
    /// > {
    ///     composable_stream::Builder {
    ///         make: move |writer: W| {
    ///             let mut builder: tar::Builder<_> = tar::Builder::new(writer);
    ///             builder.append_path(path)?;
    ///             Ok(builder)
    ///         },
    ///     }
    /// }
    ///
    /// fn compress<W: Write>(level: i32) -> composable_stream::Builder<
    ///     impl FnOnce(W) -> Result<zstd::Encoder<'static, W>, anyhow::Error>,
    /// > {
    ///     composable_stream::Builder {
    ///         make: move |writer: W| {
    ///             zstd::Encoder::new(writer, level)
    ///                 .context("Failed creating zstd encoder")
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
    /// let _file: std::fs::File = writer
    ///     .into_inner()
    ///     .context("Failed archiving")?
    ///     .finish()
    ///     .context("Failed compressing with zstd")?;
    /// #
    /// #     Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn then<A, B, C, Err>(
        self,
        other: ComposableStreamBuilder<impl FnOnce(A) -> Result<B, Err>>,
    ) -> ComposableStreamBuilder<impl FnOnce(A) -> Result<C, Err>>
    where
        M: FnOnce(B) -> Result<C, Err>,
    {
        let Self { make, .. } = self;

        ComposableStreamBuilder {
            make: move |a: A| {
                let b: B = (other.make)(a)?;
                make(b)
            },
        }
    }
}

// MARK: - Build

impl<M> ComposableStreamBuilder<M> {
    #[inline]
    pub fn build<A, Res>(self, stream: A) -> Res
    where
        M: FnOnce(A) -> Res,
    {
        (self.make)(stream)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    use self::util::*;

    #[test]
    fn test_build() -> Result<(), std::io::Error> {
        let res = source(&[1, 2, 3]).build(Vec::<u8>::new())?;

        assert_eq!(res.as_slice(), [1, 2, 3]);

        Ok(())
    }

    #[test]
    fn test_then() -> Result<(), std::io::Error> {
        let res: Vec<u8> = source(&[1, 2, 3])
            .then(ComposableStreamBuilder { make: Ok })
            .build(Vec::<u8>::new())?;

        assert_eq!(res.as_slice(), [1, 2, 3]);

        Ok(())
    }

    #[test]
    fn test_then_add_one() -> Result<(), std::io::Error> {
        let writer: AddOneWriter<Vec<u8>> =
            source(&[1, 2, 3]).then(add_one()).build(Vec::<u8>::new())?;

        let res = writer.into_inner();

        assert_eq!(res.as_slice(), [2, 3, 4]);

        Ok(())
    }

    #[test]
    fn test_then_ordering() -> Result<(), std::io::Error> {
        let writer: TimesTwoWriter<AddOneWriter<Vec<u8>>> = source(&[1, 2, 3])
            .then(times_two())
            .then(add_one())
            .build(Vec::<u8>::new())?;

        let res = writer.into_inner().into_inner();

        assert_eq!(res.as_slice(), [3, 5, 7]);

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

    pub(crate) mod util {
        use std::io::Write;

        use crate::ComposableStreamBuilder;

        pub fn unreachable<T>(err: std::convert::Infallible) -> T {
            match err {}
        }

        #[derive(Debug, Default)]
        pub struct AddOneWriter<W: Write>(W);

        impl<W: Write> Write for AddOneWriter<W> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .write(buf.iter().map(|n| n + 1).collect::<Vec<u8>>().as_slice())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                self.0.flush()
            }
        }

        impl<W: Write> AddOneWriter<W> {
            pub fn into_inner(self) -> W {
                self.0
            }
        }

        pub fn add_one<W: Write, Err>()
        -> ComposableStreamBuilder<impl FnOnce(W) -> Result<AddOneWriter<W>, Err>> {
            ComposableStreamBuilder {
                make: move |writer: W| Ok(AddOneWriter(writer)),
            }
        }

        #[derive(Debug, Default)]
        pub struct TimesTwoWriter<W: Write>(W);

        impl<W: Write> Write for TimesTwoWriter<W> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .write(buf.iter().map(|n| n * 2).collect::<Vec<u8>>().as_slice())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                self.0.flush()
            }
        }

        impl<W: Write> TimesTwoWriter<W> {
            pub fn into_inner(self) -> W {
                self.0
            }
        }

        pub fn times_two<W: Write, Err>()
        -> ComposableStreamBuilder<impl FnOnce(W) -> Result<TimesTwoWriter<W>, Err>> {
            ComposableStreamBuilder {
                make: move |writer: W| Ok(TimesTwoWriter(writer)),
            }
        }
    }
}
