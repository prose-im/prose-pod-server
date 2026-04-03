// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    io::Write,
    path::Path,
    sync::{Arc, RwLock, RwLockReadGuard},
    time::SystemTime,
};

use anyhow::Context as _;

pub const EXAMPLE_TMPDIR_VAR_NAME: &str = "EXAMPLE_TMPDIR";

pub fn init<const N: usize>(
    fs_tree: &[(&'static str, &'static str); N],
) -> Result<ExampleContext, anyhow::Error> {
    let start = SystemTime::now();

    init_tracing();

    // Create temporary directory for the example.
    let tmpdir = tempfile::TempDir::with_prefix(env!("CARGO_CRATE_NAME"))?;

    // Save temporary directory in env so it can be used by other parts of the
    // program without making the example code more verbose.
    // SAFETY: No other thread is writing or reading the environment.
    unsafe { std::env::set_var(EXAMPLE_TMPDIR_VAR_NAME, &tmpdir.path()) };

    init_fs_tree(tmpdir.path(), fs_tree).context("Failed creating fake filesystem")?;

    let tmpdir = Arc::new(RwLock::new(tmpdir));

    let old_panic_hook = std::panic::take_hook();
    std::panic::set_hook({
        let tmpdir = Arc::clone(&tmpdir);
        Box::new(move |info| {
            keep_tmpdir(&tmpdir);
            old_panic_hook(info)
        })
    });

    Ok(ExampleContext {
        start,
        tmpdir,
        cleanup_functions: Vec::new(),
    })
}

pub struct ExampleContext {
    pub start: SystemTime,
    pub tmpdir: Arc<RwLock<tempfile::TempDir>>,
    pub cleanup_functions: Vec<std::pin::Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl ExampleContext {
    pub fn tmpdir<'a>(&'a self) -> RwLockReadGuard<'a, tempfile::TempDir> {
        self.tmpdir.read().unwrap()
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::time::Uptime;

    let mut level = "info";

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--trace" => level = "trace",
            "--debug" => level = "debug",
            arg => panic!("Unknown arg: {arg:?}"),
        }
    }

    tracing_subscriber::fmt()
        .compact()
        .with_timer(Uptime::default())
        .with_target(true)
        .with_env_filter(EnvFilter::new(format!(
            "{this}={level},{this}::prose::dashboard=trace,prose_backup={level},info",
            this = env!("CARGO_CRATE_NAME")
        )))
        .init();
}

fn init_fs_tree<const N: usize>(
    fs_root: impl AsRef<Path>,
    fs_tree: &[(&'static str, &'static str); N],
) -> Result<(), anyhow::Error> {
    use std::{fs, path};

    let fs_root = fs_root.as_ref();

    tracing::debug!("Creating fake filesystem in {fs_root:?}…");

    if !fs_root.is_dir() {
        fs::create_dir_all(fs_root)?;
        tracing::trace!("+d {fs_root:?}");
    }

    let mut file: fs::File;
    for (path, contents) in fs_tree.iter() {
        let path: &path::Path = Path::new(path);

        // NOTE: If `path` is absolute, `fs_root.join(path)` results in `path`
        //   which means this would override files on your system!
        assert!(!path.is_absolute());

        let path: path::PathBuf = fs_root.join(path);

        if let Some(dir) = path.parent()
            && !dir.is_dir()
        {
            fs::create_dir_all(dir).context(format!("Dir: {dir:?}"))?;
            tracing::trace!("+d {dir:?}");
        }

        // NOTE: We could use `fs::write` here, but it uses `File::create`
        //   which _could_ override files… so let’s not do that.
        file = fs::File::create_new(&path).context(format!("File: {path:?}"))?;
        file.write_all(contents.as_bytes())
            .context(format!("File: {path:?}"))?;

        tracing::trace!("+f {path:?}");
    }

    Ok(())
}

pub fn keep_tmpdir(tmpdir: &RwLock<tempfile::TempDir>) {
    let mut tmpdir = tmpdir.write().unwrap();
    tmpdir.disable_cleanup(true);
    tracing::info!(
        "Kept fake fs root for investigation. You can find it at {path:?}.",
        path = tmpdir.path()
    );
}
