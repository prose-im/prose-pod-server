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

#[allow(dead_code)]
pub fn press_enter_to_continue() {
    tracing::info!("Press [Enter] to continue…");
    let mut buffer = String::new();
    std::io::stdin().read_line(&mut buffer).unwrap();
}

pub trait CreateBackupCommandExt<'a> {
    fn new(
        prefix: &'a str,
        description: &'a str,
        version: u8,
        blueprint: &'a prose_backup::archiving::ArchiveBlueprint,
    ) -> Self;
}

impl<'a> CreateBackupCommandExt<'a> for prose_backup::CreateBackupCommand<'a> {
    /// This is only used in examples, where rust-analyzer has feature
    /// `test` enabled for examples. It’s not the case when one _runs_
    /// examples, but rust-analyzer complains when in an IDE so this is
    /// a workaround.
    fn new(
        prefix: &'a str,
        description: &'a str,
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
