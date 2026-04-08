// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

macro_rules! env_required {
    ($name:expr) => {
        std::env::var($name).expect(&format!(
            "Environment variable `{}` should be defined",
            $name
        ))
    };
}
pub(crate) use env_required;

/// [`panic!`] in debug mode, [`tracing::error!`] in release.
macro_rules! debug_panic_or_log_error {
    ($($args:tt)*) => {
        if cfg!(debug_assertions) {
            panic!("[debug_only] {}", format!($($args)*));
        } else {
            tracing::error!($($args)*);
        }
    };
}
pub(crate) use debug_panic_or_log_error;

#[allow(dead_code)]
pub fn press_enter_to_continue() {
    tracing::info!("Press [Enter] to continue…");
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer).unwrap();
}

pub trait CreateBackupCommandExt<'a> {
    fn new<'p: 'a, 'd: 'a>(
        prefix: &'p str,
        description: &'d str,
        version: u8,
        blueprint: &'a prose_backup::archiving::ArchiveBlueprint,
    ) -> Self;
}

impl<'a> CreateBackupCommandExt<'a> for prose_backup::CreateBackupCommand<'a> {
    /// This is only used in examples, where rust-analyzer has feature
    /// `test` enabled for examples. It’s not the case when one _runs_
    /// examples, but rust-analyzer complains when in an IDE so this is
    /// a workaround.
    fn new<'p: 'a, 'd: 'a>(
        prefix: &'p str,
        description: &'d str,
        version: u8,
        blueprint: &'a prose_backup::archiving::ArchiveBlueprint,
    ) -> Self {
        Self {
            prefix,
            description,
            version,
            blueprint,
            additional_archive_data: None,
            #[cfg(feature = "test")]
            created_at: std::time::SystemTime::now(),
        }
    }
}

/// Creates a progress bar string like `━━━━━━━┈┈┈` given two `u64`.
#[allow(dead_code)]
pub fn progress_bar<const LEN: usize>(progress: u64, total: u64) -> String {
    use std::fmt::Write as _;

    let mut s = String::with_capacity(LEN);

    let filled = if total == 0 {
        0
    } else {
        // Clamp to avoid overflow past `LEN`.
        ((progress.saturating_mul(LEN as u64)) / total).min(LEN as u64) as usize
    };

    // write!(&mut s, "{0:#>1$}{0:->2$}", "", filled, LEN - filled).unwrap();
    // write!(&mut s, "{0:█>1$}{0:░>2$}", "", filled, LEN - filled).unwrap();
    // write!(&mut s, "{0:━>1$}{0:─>2$}", "", filled, LEN - filled).unwrap();
    write!(&mut s, "{0:━>1$}{0:┈>2$}", "", filled, LEN - filled).unwrap();

    s
}

macro_rules! override_files {
    ($paths:expr, in: $tmpdir:expr, to: $contents:literal) => {
        for path in $paths.iter() {
            let path = $tmpdir.path().join(path);
            std::fs::write(&path, $contents).context(format!("Failed writing in {path:?}"))?;
        }
    };
}
pub(crate) use override_files;

macro_rules! assert_file_contents {
    ($paths:expr, in: $tmpdir:expr, eq: $contents:literal) => {
        for path in $paths.iter() {
            let path = $tmpdir.path().join(path);
            let env = std::fs::read_to_string(&path).context(format!("Failed reading {path:?}"))?;
            assert_eq!(env.as_str(), $contents);
        }
    };
}
pub(crate) use assert_file_contents;
