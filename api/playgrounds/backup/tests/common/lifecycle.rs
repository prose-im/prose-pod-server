// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Once,
    time::SystemTime,
};

use crate::common::log_error;

static INIT: Once = Once::new();

/// Directory containing a fake filesystem root, to use in tests.
pub const TEST_DATA_DIR: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/data");

pub struct TestContext {
    pub now: SystemTime,
    pub test_id: String,
    pub test_data_path: PathBuf,
    pub cleanup_functions: Vec<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>>,
}

pub fn init() -> TestContext {
    INIT.call_once(|| {
        init_tracing();

        init_shared_data().unwrap();
    });

    let test_id = format!("test-{id}", id = super::unique_hex());
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

    tracing_subscriber::fmt()
        .with_test_writer()
        .compact()
        .without_time()
        .with_target(true)
        .with_env_filter(EnvFilter::new(format!(
            "{this}=trace,prose_backup=trace,trace",
            this = env!("CARGO_CRATE_NAME")
        )))
        .init();
}

fn init_shared_data() -> Result<(), anyhow::Error> {
    use anyhow::Context as _;

    tracing::info!("Creating shared test data in `{TEST_DATA_DIR}`…");

    fn exists(path: impl AsRef<Path>) -> bool {
        fs::exists(Path::new(TEST_DATA_DIR).join(path)).unwrap()
    }
    fn mkdir(path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
        let path = path.as_ref();
        if !path.is_dir() {
            fs::create_dir_all(Path::new(TEST_DATA_DIR).join(path)).context(format!(
                "Failed creating dir at '{path}'",
                path = path.display()
            ))?;
        }
        Ok(())
    }
    fn touch(path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
        let path = path.as_ref();
        if !path.is_file() {
            fs::File::create(Path::new(TEST_DATA_DIR).join(path)).context(format!(
                "Failed creating file at '{path}'",
                path = path.display()
            ))?;
        }
        Ok(())
    }

    let test_data_dir = Path::new(TEST_DATA_DIR);

    mkdir("foo")?;
    touch("foo/a")?;
    touch("foo/b")?;

    mkdir("bar")?;
    touch("bar/a")?;
    touch("bar/b")?;

    mkdir("baz")?;
    if !exists("baz/example.bin") {
        Command::new("dd")
            .arg("if=/dev/zero")
            .arg("of=baz/example.bin")
            .arg("bs=1K")
            .arg("count=12")
            .current_dir(test_data_dir)
            .output()
            .unwrap();
    }

    Ok(())
}
