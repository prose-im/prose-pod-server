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
            additional_archive_data: Vec::with_capacity(0),
            #[cfg(feature = "test")]
            created_at: std::time::SystemTime::now(),
        }
    }
}

/// Creates a progress bar string like `━━━━━━━┈┈┈` given two `u64`.
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
