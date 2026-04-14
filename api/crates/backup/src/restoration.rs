// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::path::PathBuf;

use anyhow::Context as _;

use crate::archiving::{ArchiveBlueprint, ExtractionOutput};
use crate::{BackupId, RestoreBackupEventHandler};

pub(crate) use self::RestorationContext as Context;

#[derive(Debug, Default)]
pub struct RestorationContext {
    /// WARN: This `Vec` MUST always be sorted.
    pub migrations: Vec<ArchiveMigration>,
}

#[derive(Clone)]
pub struct ArchiveMigration {
    pub version: u8,
    pub migrate_paths: Vec<(String, String)>,
}

impl ArchiveMigration {
    pub fn new<Dst, Src>(version: u8, migrate_paths: impl IntoIterator<Item = (Dst, Src)>) -> Self
    where
        Dst: ToString,
        Src: ToString,
    {
        Self {
            version,
            migrate_paths: migrate_paths
                .into_iter()
                .map(|(dst, src)| (dst.to_string(), src.to_string()))
                .collect(),
        }
    }
}

#[derive(Debug)]
pub struct RestorationOutput;

#[derive(Debug, thiserror::Error)]
pub enum RestorationError {
    #[error("Extraction failed")]
    ExtractionFailed(#[from] crate::archiving::ExtractionError),

    #[error("Move failed from `{from}` to `{to}`")]
    MoveFailed {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: anyhow::Error,
    },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub(crate) fn restore<'a>(
    backup_id: &BackupId,
    ExtractionOutput {
        tmp_dir, metadata, ..
    }: ExtractionOutput<'a>,
    blueprint: &ArchiveBlueprint,
    context: &RestorationContext,
    event_handler: &mut impl RestoreBackupEventHandler,
) -> Result<RestorationOutput, RestorationError> {
    use crate::util::{PathGuard, safe_replace};

    tracing::debug!("Restoring with: {blueprint:#?}");

    // Migrate data if needed.
    if metadata.version < blueprint.version {
        for migration in filter_migrations(&context.migrations, metadata.version, blueprint.version)
        {
            migrate(migration, tmp_dir.path())?;
        }
    }

    let mut processed: Vec<(PathBuf, PathBuf, Option<PathGuard>)> = Vec::new();

    for (dir_name, dst) in blueprint.paths.iter() {
        let src = tmp_dir.path().join(dir_name);

        match safe_replace(&src, &dst) {
            Ok(backup_guard) => {
                processed.push((src, dst.to_owned(), backup_guard));
                event_handler.on_path_restored(backup_id, dst.as_path());
            }
            Err(err) => {
                // Revert previous operations.
                revert(processed);

                // Abort backup restoration.
                return Err(RestorationError::MoveFailed {
                    from: src,
                    to: dst.to_owned(),
                    source: anyhow::Error::new(err),
                });
            }
        };
    }

    let entries = std::fs::read_dir(tmp_dir.path())
        .context("Failed reading extraction temporary directory")?;
    for entry in entries {
        match entry {
            Ok(entry) => {
                let file_name = entry.file_name();
                tracing::warn!("Extracted unknown entry {file_name:?}.")
            }
            Err(err) => tracing::error!("{err:?}"),
        }
    }

    Ok(RestorationOutput)
}

// MARK: - Helpers

fn filter_migrations<'a>(
    migrations: impl IntoIterator<Item = &'a ArchiveMigration>,
    from: u8,
    to: u8,
) -> impl Iterator<Item = &'a ArchiveMigration> {
    migrations
        .into_iter()
        .skip_while(move |m| m.version < from)
        .take_while(move |m| m.version <= to)
}

fn migrate(
    migration: &ArchiveMigration,
    tmp_dir: &std::path::Path,
) -> Result<(), RestorationError> {
    tracing::debug!("Applying migration: {migration:#?}");

    for (from_key, to_key) in migration.migrate_paths.iter() {
        let from = tmp_dir.join(from_key);
        let to = tmp_dir.join(to_key);

        let from_metadata = match from.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::debug!("Cannot read {from:?} metadata, skipping. (Error: {err:?})");
                continue;
            }
        };

        // NOTE: Not using `Path::metadata` in case `to` doesn’t exist.
        if to.is_dir() {
            if from_metadata.is_dir() {
                // Merge files into destination.
                let entries =
                    std::fs::read_dir(&from).with_context(|| format!("Cannot walk {from:?}"))?;
                for entry in entries {
                    let entry = entry.with_context(|| format!("Invalid entry in {from:?}"))?;
                    let file_name = entry.file_name();
                    let from = from.join(&file_name);
                    let to = to.join(&file_name);
                    std::fs::rename(&from, &to).with_context(|| {
                        format!("Could not migrate (dir -> dir) {from:?} to {to:?}")
                    })?;
                }
            } else {
                let to = to.join(from_key);
                std::fs::rename(&from, &to).with_context(|| {
                    format!("Could not migrate (file -> dir) {from:?} to {to:?}")
                })?;
            }
        } else {
            std::fs::rename(&from, &to)
                .with_context(|| format!("Could not migrate {from:?} to {to:?}"))?;
        }
    }

    Ok(())
}

/// Note that this is best-effort, meaning we’re already doing error recovery
/// at this point so we can’t recover from subsequent internal errors.
#[cold]
fn revert(processed: Vec<(PathBuf, PathBuf, Option<crate::util::PathGuard>)>) {
    use std::fs;

    for (tmp_path, replaced, original) in processed {
        // Move new file back into its original location.
        if let Err(err) = fs::rename(&replaced, &tmp_path) {
            tracing::error!(
                "Could not revert `{path}`: {err:?}",
                path = replaced.display()
            );
            continue;
        }

        // Recover backed up file/directory.
        if let Some(original) = original {
            if let Err(err) = fs::rename(&original, &replaced) {
                tracing::error!(
                    "Could not recover `{path}`: {err:?}",
                    path = replaced.display()
                )
            };
        }
    }
}

// MARK: - Boilerplate

impl std::fmt::Debug for ArchiveMigration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            version,
            migrate_paths,
        } = self;

        f.debug_struct("ArchiveMigration")
            .field("version", version)
            .field("migrate_paths", &crate::util::fmt::AsMap(migrate_paths))
            .finish()
    }
}
