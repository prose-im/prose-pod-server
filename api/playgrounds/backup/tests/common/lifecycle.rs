// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Once,
    time::SystemTime,
};

use crate::common::log_error;

static INIT: Once = Once::new();

pub struct TestContext {
    pub now: SystemTime,
    pub test_id: String,
    pub test_data_path: PathBuf,
    pub cleanup_functions: Vec<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>>,
}

pub fn init() -> TestContext {
    INIT.call_once(|| {
        init_tracing();
    });

    let test_id = format!("test-{id}", id = super::unique_hex().unwrap());
    tracing::debug!("Test id: {test_id}");

    let test_data_path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(&test_id);
    tracing::debug!("Will save test data in `{}`.", test_data_path.display());

    TestContext {
        now: SystemTime::now(),
        test_data_path,
        test_id,
        cleanup_functions: Vec::new(),
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        println!();

        // Run cleanup functions in reverse order.
        let cleanup_functions = std::mem::take(&mut self.cleanup_functions);
        tokio::task::block_in_place(move || {
            tokio::runtime::Handle::current().block_on(async move {
                for func in cleanup_functions.into_iter().rev() {
                    func.await
                }
            })
        });

        let test_failed = std::thread::panicking();

        // Do not cleanup on test failures.
        if test_failed || std::env::var("NO_CLEANUP").is_ok() {
            tracing::info!(
                "Test data can be found in `{test_data_path}` (not cleaning up).",
                test_data_path = self.test_data_path.display()
            );

            if self.test_data_path.join("cache").is_dir() {
                if let Err(err) = std::fs::rename(
                    self.test_data_path.join("cache"),
                    self.test_data_path.join("cache.bak"),
                ) {
                    tracing::warn!(
                        "Could not prevent the cache directory from being \
                        automatically deleted: {err:?}"
                    );
                }
            }

            return;
        }

        if self.test_data_path.exists() {
            tracing::info!(
                "Cleaning up test data in `{test_data_path}` (set `NO_CLEANUP` to avoid this)…",
                test_data_path = self.test_data_path.display()
            );

            fs::remove_dir_all(&self.test_data_path).unwrap_or_else(log_error!());
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::time::uptime;

    tracing_subscriber::fmt()
        .with_test_writer()
        .compact()
        // .without_time()
        .with_timer(uptime())
        .with_target(true)
        .with_env_filter(EnvFilter::new(format!(
            "{this}=trace,prose_backup=trace,info",
            this = env!("CARGO_CRATE_NAME")
        )))
        .init();
}
