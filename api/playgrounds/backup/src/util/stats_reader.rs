// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    io::{self, Read},
    time::SystemTime,
};

// MARK: Stats

pub struct ReadStats {
    bytes_read: u64,
    read_calls: u64,
    duration: std::time::Duration,
}

impl ReadStats {
    pub fn new() -> Self {
        Self {
            bytes_read: 0,
            read_calls: 0,
            duration: std::time::Duration::ZERO,
        }
    }

    #[allow(unused)]
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    #[allow(unused)]
    pub fn read_calls(&self) -> u64 {
        self.read_calls
    }
}

impl std::fmt::Display for ReadStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{bytes}B", bytes = self.bytes_read)?;

        write!(f, " in {}ms", self.duration.as_secs_f32() * 1000.)?;

        write!(
            f,
            " ({calls} {chunks_str})",
            calls = self.read_calls,
            chunks_str = if self.read_calls < 2 {
                "chunk"
            } else {
                "chunks"
            }
        )
    }
}

// MARK: Reader

/// A `Read` wrapper that stores stats (e.g. bytes read, number of `read`
/// calls…).
pub struct StatsReader<'r, R> {
    inner: R,
    stats: &'r mut ReadStats,
}

impl<'r, R: Read> StatsReader<'r, R> {
    pub fn new(inner: R, stats: &'r mut ReadStats) -> Self {
        Self { inner, stats }
    }
}

impl<'r, R: Read> Read for StatsReader<'r, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let start = SystemTime::now();

        let n = self.inner.read(buf)?;

        self.stats.bytes_read += n as u64;
        self.stats.read_calls += 1;

        let end = SystemTime::now();
        if let Ok(duration) = end.duration_since(start) {
            self.stats.duration += duration;
        }

        Ok(n)
    }
}
