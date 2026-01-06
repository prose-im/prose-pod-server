// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::{self, Write};

/// A [`Write`]r that duplicates input into two underlying `Write`rs.
///
/// See [“tee (command)” on Wikipedia][tee] for more information about the
/// naming and use cases.
///
/// [tee]: https://en.wikipedia.org/wiki/Tee_(command)
pub struct TeeWriter<W1, W2> {
    pub a: W1,
    pub b: W2,
}

impl<W1, W2> TeeWriter<W1, W2> {
    pub fn new(a: W1, b: W2) -> Self {
        Self { a, b }
    }
}

impl<W1: Write, W2: Write> Write for TeeWriter<W1, W2> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Write to first writer
        let n = self.a.write(buf)?;

        // Write the same amount to the second writer
        // If this fails, return that error
        self.b.write_all(&buf[..n])?;

        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.a.flush()?;
        self.b.flush()?;
        Ok(())
    }
}
