// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! A reader used to measure stats when performing read operations.
//!
//! It measures things like the number of bytes read and the time spent reading.

use std::io::{self, Read, Write};

use writer_chain::WriterChainBuilder;

// MARK: Model

pub trait StreamStats {
    fn record_chunk(&mut self, len: usize);

    // NOTE: Do not record active time in release builds, it would just add
    //   unnecessary overhead.
    #[cfg(debug_assertions)]
    fn record_duration(&mut self, duration: &std::time::Duration);
}

pub trait WriterStats: StreamStats {
    fn record_flush(&mut self);
}

// MARK: Reader

/// A `Read`/`Write` wrapper that stores stats (e.g. bytes read/written,
/// number of `read`/`write` calls…).
pub struct MeteredStream<Stream, Stats: StreamStats> {
    inner: Stream,
    stats: Stats,
}

impl<Stream, Stats: StreamStats> MeteredStream<Stream, Stats> {
    pub fn new(inner: Stream, stats: Stats) -> Self {
        Self { inner, stats }
    }

    pub fn into_inner(self) -> Stream {
        self.inner
    }
}

impl<Stream, Stats: StreamStats> Read for MeteredStream<Stream, Stats>
where
    Stream: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(debug_assertions)]
        let start = std::time::SystemTime::now();

        let n = self.inner.read(buf)?;

        #[cfg(debug_assertions)]
        {
            let end = std::time::SystemTime::now();
            if let Ok(duration) = end.duration_since(start) {
                self.stats.record_duration(&duration);
            }
        }

        self.stats.record_chunk(n);

        Ok(n)
    }
}

impl<Stream, Stats: StreamStats> Write for MeteredStream<Stream, Stats>
where
    Stream: Write,
    Stats: WriterStats,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        #[cfg(debug_assertions)]
        let start = std::time::SystemTime::now();

        let n = self.inner.write(buf)?;

        #[cfg(debug_assertions)]
        {
            let end = std::time::SystemTime::now();
            if let Ok(duration) = end.duration_since(start) {
                self.stats.record_duration(&duration);
            }
        }

        self.stats.record_chunk(n);

        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

// MARK: DTOs

macro_rules! gen_stats_dto {
    ($t:ident {
        $bytes_count:ident,
        $chunks_count:ident,
        $active_duration:ident
        $(, $field:ident: $field_type:ty)*
        $(,)?
    }) => {
        #[derive(Debug)]
        pub struct $t {
            pub $bytes_count: u64,
            pub $chunks_count: u64,
            #[cfg(debug_assertions)]
            pub $active_duration: std::time::Duration,
            $(pub $field: $field_type,)*
        }

        impl $t {
            #[inline(always)]
            pub fn new() -> Self {
                Self {
                    $bytes_count: 0,
                    $chunks_count: 0,
                    #[cfg(debug_assertions)]
                    $active_duration: std::time::Duration::ZERO,
                    $($field: Default::default(),)*
                }
            }
        }

        impl Default for $t {
            #[inline(always)]
            fn default() -> Self {
                Self::new()
            }
        }

        impl StreamStats for $t {
            fn record_chunk(&mut self, len: usize) {
                self.$bytes_count += len as u64;
                self.$chunks_count += 1;
            }

            #[cfg(debug_assertions)]
            fn record_duration(&mut self, duration: &std::time::Duration) {
                self.$active_duration += *duration;
            }
        }

        impl std::fmt::Display for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{bytes}B", bytes = self.$bytes_count)?;

                #[cfg(debug_assertions)]
                write!(f, " in {}ms", self.$active_duration.as_secs_f32() * 1000.)?;

                write!(
                    f,
                    " ({calls} {chunks_str})",
                    calls = self.$chunks_count,
                    chunks_str = if self.$chunks_count < 2 {
                        "chunk"
                    } else {
                        "chunks"
                    }
                )
            }
        }
    };
}

gen_stats_dto!(ReadStats {
    bytes_read,
    chunks_read,
    active_read_duration,
});

gen_stats_dto!(WriteStats {
    bytes_written,
    chunks_written,
    active_write_duration,
    flush_count: u32,
});

impl WriterStats for WriteStats {
    fn record_flush(&mut self) {
        self.flush_count += 1;
    }
}

// MARK: Convenience helpers.

pub(crate) fn meter_writes<W, MakeErr, FinalizeErr, Stats: WriterStats>(
    stats: Stats,
) -> WriterChainBuilder<
    impl FnOnce(W) -> Result<MeteredStream<W, Stats>, MakeErr>,
    impl FnOnce(MeteredStream<W, Stats>) -> Result<(W, Stats), FinalizeErr>,
> {
    WriterChainBuilder {
        make: move |writer: W| Ok(MeteredStream::new(writer, stats)),
        finalize: move |writer: MeteredStream<W, Stats>| Ok((writer.inner, writer.stats)),
    }
}

// MARK: - Boilerplate

impl<T: StreamStats> StreamStats for &mut T {
    #[inline(always)]
    fn record_chunk(&mut self, len: usize) {
        (*self).record_chunk(len)
    }

    #[cfg(debug_assertions)]
    #[inline(always)]
    fn record_duration(&mut self, duration: &std::time::Duration) {
        (*self).record_duration(duration)
    }
}

impl<T: WriterStats> WriterStats for &mut T {
    #[inline(always)]
    fn record_flush(&mut self) {
        (*self).record_flush()
    }
}
