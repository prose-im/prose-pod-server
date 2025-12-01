// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // NOTE: This `bin` target is just a thin wrapper around a `lib` target
    //   because crates that invoke the system linker (e.g. `bin` crates)
    //   cannot be cached by sccache (which we use to speed builds up). See
    //   [mozilla/sccache](https://github.com/mozilla/sccache?tab=readme-ov-file#rust).
    prose_pod_server::main().await
}
