// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::path::PathBuf;

use tempfile::TempDir;

use crate::archiving::ExtractionOutput;

#[derive(Debug)]
pub struct RestorationOutput;

pub(crate) fn restore<'a>(
    ExtractionOutput {
        tmp_dir, blueprint, ..
    }: ExtractionOutput<'a>,
) -> Result<RestorationOutput, RestorationError> {
    use crate::util::{PathGuard, safe_replace};

    let mut processed: Vec<(PathBuf, PathBuf, Option<PathGuard>)> = Vec::new();

    for (dir_name, dst) in blueprint.paths.iter() {
        let src = tmp_dir.path().join(dir_name);

        match safe_replace(&src, &dst) {
            Ok(backup_guard) => processed.push((src, dst.to_owned(), backup_guard)),
            Err(err) => {
                // Revert previous operations.
                revert(processed);

                // Abort backup restoration.
                return Err(RestorationError::MoveFailed {
                    tmp_dir,
                    source: anyhow::Error::new(err),
                });
            }
        };
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
            fs::rename(&original, &replaced).unwrap_or_else(|err| {
                tracing::error!(
                    "Could not recover `{path}`: {err:?}",
                    path = replaced.display()
                )
            });
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RestorationError {
    #[error("Extraction failed")]
    ExtractionFailed(#[from] crate::archiving::ExtractionError),

    #[error("Move failed")]
    MoveFailed {
        tmp_dir: TempDir,
        #[source]
        source: anyhow::Error,
    },
}
