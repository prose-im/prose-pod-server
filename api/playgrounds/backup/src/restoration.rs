// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::path::PathBuf;

use anyhow::Context as _;

use crate::archiving::{ArchiveBlueprint, ExtractionOutput};
use crate::{BackupId, RestoreBackupEventHandler};

#[derive(Debug)]
pub struct RestorationOutput;

pub(crate) fn restore<'a>(
    backup_id: &BackupId,
    ExtractionOutput { tmp_dir, .. }: ExtractionOutput<'a>,
    blueprint: &ArchiveBlueprint,
    event_handler: &mut impl RestoreBackupEventHandler,
) -> Result<RestorationOutput, RestorationError> {
    use crate::util::{PathGuard, safe_replace};

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
