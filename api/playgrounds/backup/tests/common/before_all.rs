// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{fs, path::Path, process::Command, sync::Once, time::SystemTime};

use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

/// Directory containing a fake filesystem root, to use in tests.
pub const TEST_DATA_DIR: &'static str = "./data";

pub fn init() -> (String, SystemTime) {
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

    let test_id = super::unique_hex();
    tracing::info!("Test id: {test_id}");

    return (test_id, SystemTime::now());
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
