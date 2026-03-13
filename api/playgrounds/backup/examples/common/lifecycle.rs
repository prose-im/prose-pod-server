// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Once;

static INIT: Once = Once::new();

pub fn init() {
    INIT.call_once(|| {
        init_tracing();
    });
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::time::Uptime;

    tracing_subscriber::fmt()
        .compact()
        .with_timer(Uptime::default())
        .with_target(true)
        .with_env_filter(EnvFilter::new(format!(
            "{this}=trace,prose_backup=trace,info",
            this = env!("CARGO_CRATE_NAME")
        )))
        .init();
}
