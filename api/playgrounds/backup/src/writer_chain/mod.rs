// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod either;
pub mod tee;

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
    #[must_use]
    pub fn build<InnermostWriter, OuterWriter, Out, Err>(
        self,
        writer: InnermostWriter,
    ) -> Result<(OuterWriter, F), Err>
    where
        InnermostWriter: std::io::Write,
        M: FnOnce(InnermostWriter) -> Result<OuterWriter, Err>,
        F: FnOnce(OuterWriter) -> Out,
    {
        let Self { make, finalize, .. } = self;

        make(writer).map(move |w| (w, finalize))
    }
}
