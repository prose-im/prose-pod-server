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

use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

/// Directory containing a fake filesystem root, to use in tests.
pub const TEST_DATA_DIR: &'static str = concat!(env!("CARGO_TARGET_TMPDIR"), "/data");

pub struct TestContext {
    pub now: SystemTime,
    pub test_id: String,
    pub test_data_path: PathBuf,
}

pub fn init() -> TestContext {
    init_tracing();

    INIT.call_once(|| {
        tracing::warn!("Creating test data in `{TEST_DATA_DIR}`…");

        fn exists(path: impl AsRef<Path>) -> bool {
            fs::exists(Path::new(TEST_DATA_DIR).join(path)).unwrap()
        }
        fn mkdir(path: impl AsRef<Path>) {
            fs::create_dir_all(Path::new(TEST_DATA_DIR).join(path)).unwrap();
        }
        fn touch(path: impl AsRef<Path>) {
            fs::File::create(Path::new(TEST_DATA_DIR).join(path)).unwrap();
        }

        let test_data_dir = Path::new(TEST_DATA_DIR);

        mkdir("etc/prosody");

        if !exists("etc/prosody/prosody.cfg.lua") {
            Command::new("wget")
                .arg("-O")
                .arg("etc/prosody/prosody.cfg.lua")
                .arg("https://raw.githubusercontent.com/prose-im/prose-pod-system/refs/heads/master/server/local/etc/prosody/prosody.cfg.lua")
                .current_dir(test_data_dir)
                .output()
                .unwrap();
        }

        mkdir("var/lib/prosody");

        mkdir("var/lib/prosody/example%2eorg");
        touch("var/lib/prosody/example%2eorg/cron.dat");

        mkdir("var/lib/prosody/example%2eorg/accounts");
        touch("var/lib/prosody/example%2eorg/accounts/pauline%2ecollins.dat");

        mkdir("var/lib/prosody/example%2eorg/upload%2eprose%2elocal");
        touch("var/lib/prosody/example%2eorg/upload%2eprose%2elocal/cron.dat");

        if !exists("var/lib/prosody/upload%2eprose%2elocal/example.bin") {
            Command::new("dd")
                .arg("if=/dev/zero")
                .arg("of=var/lib/prosody/upload%2eprose%2elocal/example.bin")
                .arg("bs=1K")
                .arg("count=12")
                .current_dir(test_data_dir)
                .output()
                .unwrap();
        }
    });

    let test_id = format!("test-{id}", id = super::unique_hex());
    tracing::debug!("Test id: {test_id}");

    let test_data_path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(&test_id);
    tracing::debug!("Will save test data in `{}`.", test_data_path.display());

    TestContext {
        now: SystemTime::now(),
        test_data_path,
        test_id,
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        let test_failed = std::thread::panicking();

        // Do not cleanup on test failures.
        if test_failed || std::env::var("NO_CLEANUP").is_ok() {
            tracing::info!(
                "Test data can be found in `{test_data_path}` (not cleaning up).",
                test_data_path = self.test_data_path.display()
            );
            return;
        }

        tracing::info!(
            "Cleaning up test data in `{test_data_path}` (set `NO_CLEANUP` to avoid this)…",
            test_data_path = self.test_data_path.display()
        );

        fn log_error<E: std::fmt::Display>(error: E) {
            tracing::error!("{error:#}");
        }

        fs::remove_dir_all(&self.test_data_path).unwrap_or_else(log_error);
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .compact()
        .without_time()
        .with_target(true)
        .with_env_filter(EnvFilter::new(format!(
            "{this}=trace,prose_backup=trace,info",
            this = env!("CARGO_CRATE_NAME")
        )))
        .init();
}
