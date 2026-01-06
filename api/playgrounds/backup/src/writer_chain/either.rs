// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub enum Either<A, B> {
    A(A),
    B(B),
}

impl<W1, W2> std::io::Write for Either<W1, W2>
where
    W1: std::io::Write,
    W2: std::io::Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Either::A(writer) => writer.write(buf),
            Either::B(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Either::A(writer) => writer.flush(),
            Either::B(writer) => writer.flush(),
        }
    }
}
