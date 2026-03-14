// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::time::SystemTime;

pub const EXAMPLE_TMPDIR_VAR_NAME: &'static str = "EXAMPLE_TMPDIR";

pub fn init() -> Result<ExampleContext, anyhow::Error> {
    let start = SystemTime::now();

    init_tracing();

    // Create temporary directory for the example.
    let tmpdir = tempfile::TempDir::with_prefix(env!("CARGO_CRATE_NAME"))?;

    // Save temporary directory in env so it can be used by other parts of the
    // program without making the example code more verbose.
    // SAFETY: No other thread is writing or reading the environment.
    unsafe { std::env::set_var(EXAMPLE_TMPDIR_VAR_NAME, &tmpdir.path()) };

    Ok(ExampleContext {
        start,
        tmpdir,
        cleanup_functions: Vec::new(),
    })
}

pub struct ExampleContext {
    pub start: SystemTime,
    pub tmpdir: tempfile::TempDir,
    pub cleanup_functions: Vec<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>>,
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
