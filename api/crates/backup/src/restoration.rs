// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use anyhow::{Context as _, anyhow};

#[cfg(debug_assertions)]
use crate::archiving::log_extracted_entry;
use crate::archiving::{
    ArchiveBlueprint, BackupInternalMetadata, ExtractBackupEventHandler, ExtractionReport,
    archive_reader, read_metadata,
};
use crate::decryption::{DecryptionContext, DecryptionReport};
use crate::stats::{MeteredStream, ReadStats, StreamStats};
use crate::util::{self, concat_byte_slices, concat_osstr, debug_panic, is_same_device};
use crate::verification::VerificationOutput;
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
pub struct RestorationOutput {
    /// Metadata stored inside of the backup.
    #[allow(dead_code)]
    pub(crate) metadata: BackupInternalMetadata,

    pub additional_data: Option<(tempfile::TempDir, RestoreRevertGuard)>,
}

#[derive(Debug, thiserror::Error)]
pub enum RestorationError {
    #[error("Could not backup `{path}` before restoration (to prevent data loss)")]
    PathBackupFailed {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },

    #[error("Extraction failed")]
    ExtractionFailed(#[from] ExtractionError),

    #[error("Found unexpected data. This is a logic error.")]
    FoundUnexpectedData(Vec<PathBuf>),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<std::io::Error> for RestorationError {
    fn from(error: std::io::Error) -> Self {
        Self::ExtractionFailed(ExtractionError::from(error))
    }
}

pub(crate) fn restore(
    backup_id: &BackupId,
    VerificationOutput { backup_path, .. }: &VerificationOutput,
    blueprint: &ArchiveBlueprint,
    context: &RestorationContext,
    decryption_context: &DecryptionContext,
    blueprints: &HashMap<u8, ArchiveBlueprint>,
    event_handler: &mut impl RestoreBackupEventHandler,
) -> Result<RestorationOutput, RestorationError> {
    use std::collections::HashSet;

    tracing::debug!(?backup_id, "Restoring with: {blueprint:#?}");

    let backup_size = backup_path
        .metadata()
        .context("Could not read backup file metadata")
        .inspect_err(debug_panic)?
        .len();
    event_handler.on_restoration_start(backup_id, backup_size);

    let backup_file = std::fs::File::open(backup_path.as_path())
        .context("Could not open backup file")
        .inspect_err(debug_panic)?;

    let backup_reader = MeteredStream::new(
        backup_file,
        RawReadStats {
            backup_id,
            event_handler,
        },
    );

    let mut decryption_report = DecryptionReport::default();
    let mut decryption_stats = ReadStats::default();
    let mut decompression_stats = ReadStats::default();
    let mut archive = archive_reader(
        backup_reader,
        backup_id,
        decryption_context,
        &mut decryption_stats,
        &mut decryption_report,
        &mut decompression_stats,
    )?;

    let mut entries = archive.entries().map_err(anyhow::Error::from)?;

    let mut extraction_report = ExtractionReport::default();
    let metadata = read_metadata(&mut entries, backup_id, &mut extraction_report)?;

    // Find where to extract entries based on the backup version.
    // NOTE: This is done before computing-heavy tasks, to fail faster.
    let Some(backup_blueprint) = blueprints.get(&metadata.version) else {
        return Err(RestorationError::ExtractionFailed(
            ExtractionError::UnknownBackupVersion(metadata.version),
        ));
    };
    tracing::debug!(?backup_id, "Extracting with: {backup_blueprint:#?}");

    // Compute path mappings.
    let migrations = {
        // TODO: Support reverse migrations.
        if metadata.version < blueprint.version {
            let migrations =
                filter_migrations(&context.migrations, metadata.version, blueprint.version)
                    .flat_map(|migration| migration.migrate_paths.iter());
            flatten(migrations)
        } else {
            Vec::with_capacity(0)
        }
    };

    // Sort mappings so the longer paths are first.
    // NOTE: This is important in case the blueprint specifies e.g. `foo/`
    //   then an “override” for `foo/a` (in this order).
    let path_mappings = {
        let mut paths = blueprint.paths.clone();
        paths.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()));
        tracing::debug!(?backup_id, "Path mappings: {:#?}", util::fmt::AsMap(&paths));
        paths
    };

    // Backup destination paths to revert in case an error happens.
    let mut revert_guard = backup_destinations(path_mappings.iter())?;

    // Store in a boolean if an entry was extracted in the temporary directory.
    // This saves us from having to read the temporary directory to check if
    // it’s empty or not.
    let mut restoration_is_partial = false;

    // Extract the archive.
    // NOTE: Most paths will be mapped outside of the temporary directory.
    //   Only unknown data will be unpacked in this directory. This allows
    //   for additional data that’s not in the blueprint (required use case).
    let tmp_dir = tempfile::TempDir::new()
        .context("Could not create temporary directory to extract the backup in")
        .map_err(ExtractionError::Other)?;
    tracing::debug!(
        ?backup_id,
        "Extracting backup in `{path}`…",
        path = tmp_dir.path().display()
    );
    for entry in entries {
        let mut entry = entry?;

        let original_path = entry.path()?.to_path_buf();

        let dst_opt = map_path(&mut entry, migrations.iter(), path_mappings.iter());
        let dst = match dst_opt {
            Some(ref dst) => dst.as_path(),
            None => {
                restoration_is_partial = true;
                tmp_dir.path()
            }
        };

        // Unpack the archive entry.
        entry.unpack_in(dst).with_context(|| {
            format!(
                "Failed extracting {original_path:?} as {entry_path:?} in {dst:?}",
                entry_path = entry
                    .path()
                    .map_or_else(|e| format!("Err({e:?})"), |p| p.display().to_string()),
            )
        })?;

        if let Ok(entry_size) = entry.header().entry_size() {
            extraction_report.on_extraction_progress(backup_id, entry_size);
        }

        #[cfg(debug_assertions)]
        log_extracted_entry(&entry)?;
    }
    drop(archive);

    // Make sure all expected paths were present.
    {
        let mut missing_paths: HashSet<&PathBuf> = HashSet::new();

        for (_, dst) in path_mappings.iter() {
            if !dst.exists() {
                missing_paths.insert(dst);
            }
        }

        if !missing_paths.is_empty() {
            return Err(RestorationError::ExtractionFailed(
                ExtractionError::InvalidBackup(anyhow!("Missing data ({missing_paths:?}).")),
            ));
        }
    }

    event_handler.on_decryption_finished(backup_id, decryption_stats, decryption_report);
    event_handler.on_decompression_finished(backup_id, decompression_stats);
    event_handler.on_extraction_finished(backup_id, extraction_report);

    let additional_data = if restoration_is_partial {
        tracing::debug!(
            ?backup_id,
            path =? tmp_dir.path().display().to_string(),
            "Extraction finished, but additional data needs to be processed \
            to finish restoration."
        );

        Some((tmp_dir, revert_guard))
    } else {
        tracing::debug!(?backup_id, "Restoration finished.");

        revert_guard.defuse();

        None
    };

    Ok(RestorationOutput {
        metadata,
        additional_data,
    })
}

// MARK: - Helpers

fn filter_migrations<'a>(
    migrations: impl IntoIterator<Item = &'a ArchiveMigration>,
    from: u8,
    to: u8,
) -> impl Iterator<Item = &'a ArchiveMigration> {
    migrations
        .into_iter()
        .skip_while(move |&m| m.version <= from)
        .take_while(move |&m| m.version <= to)
}

/// Flatten migrations (e.g. `[(v2 -> v3), (v3 -> v4)] = [(v2 -> v4)]`).
///
/// NOTE: This is O(n²/2), but tends to O(n) as more migrations are added.
#[must_use]
fn flatten<'a, 'b, S: AsRef<OsStr> + 'a>(
    migrations: impl Iterator<Item = &'a (S, S)>,
) -> Vec<(Box<OsStr>, Box<OsStr>)> {
    use std::os::unix::ffi::OsStrExt as _;

    let mut flat_migrations: Vec<(Box<OsStr>, Box<OsStr>)> =
        Vec::with_capacity(migrations.size_hint().0);

    for (new_from, new_to) in migrations {
        let new_from: &OsStr = new_from.as_ref();
        let new_to: &OsStr = new_to.as_ref();
        let mut need_insert = true;

        for (_, to) in flat_migrations.iter_mut() {
            let mut new_from = new_from.as_bytes();

            // If `new_from` ends with a `/`, ignore it. It simplifies further logic.
            // PERF: This avoids allocating a new `Vec` with `/` as suffix.
            if new_from.ends_with(b"/") {
                new_from = &new_from[..new_from.len() - 1];
            }

            if let Some(suffix) = to.as_bytes().strip_prefix(new_from) {
                if suffix.is_empty() || suffix == b"/" {
                    // Exact match.
                    tracing::trace!("Mapped {to:?} to {new_to:?}");
                    *to = Box::from(new_to);
                    need_insert = false;
                    break;
                } else if suffix.starts_with(b"/") {
                    // Proper prefix.
                    let new_path = concat_osstr(new_to, OsStr::from_bytes(suffix));
                    tracing::trace!("Mapped {to:?} to {new_path:?}");
                    *to = Box::from(new_path.as_os_str());
                    break;
                } else {
                    // Not a real prefix (e.g. `abc` matches `abcd/ef`),
                    // but it really is a different directory.
                    continue;
                }
            }
        }

        if need_insert {
            flat_migrations.push((Box::from(new_from), Box::from(new_to)));
        }
    }

    // Sort mappings so the longer paths are first.
    flat_migrations.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()));

    tracing::debug!("Migrations: {:#?}", util::fmt::AsMap(&flat_migrations));

    flat_migrations
}

/// This function changes the path of an entry according to path mappings.
///
/// Because the `tar` crate ignores “root” path components (i.e. leading `/`),
/// we can’t just map the path as it would be extracted in the unpacking
/// destination directory and not `/`. For security reasons, the `tar` crate
/// skips entries which would end up outside of the destination directory. This
/// is great and we must not work around this feature.
///
/// By returning a new destination path if the entry is expected (i.e. prefix
/// in path map), we ensure safe unpacking while still working around unwanted
/// safety features.
///
/// Here are examples (pseudo-code):
///
/// ```txt
/// map_path(Entry("foo/bar"), [], [("foo/", "/var/lib/foo/")])
/// -> entry = Entry("bar"), res = Some("/var/lib/foo/")
///
/// map_path(Entry("baz"), [], [("foo/", "/var/lib/foo/")])
/// -> entry = Entry("baz"), res = None
///
/// map_path(Entry("foo/bar"), [], [("foo/bar", "/var/lib/foo/bar")])
/// -> entry = Entry("."), res = Some("/var/lib/foo/bar")
/// ```
///
/// Also see <https://github.com/alexcrichton/tar-rs/issues/335> for additional
/// limitations of the `tar` crate.
#[must_use]
fn map_path<'a, 'b, R: std::io::Read>(
    entry: &mut tar::Entry<R>,
    migrations: impl Iterator<Item = &'a (Box<OsStr>, Box<OsStr>)>,
    path_mappings: impl Iterator<Item = &'b (OsString, PathBuf)>,
) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::Path;

    let original_path = entry.path_bytes();
    let mut new_path = original_path.to_vec();

    // Apply migrations.
    for (from, to) in migrations {
        let mut from = from.as_bytes();
        let to = to.as_bytes();

        // If `from` ends with a `/`, ignore it. It simplifies further logic.
        // PERF: This avoids allocating a new `Vec` with `/` as suffix.
        if from.ends_with(b"/") {
            from = &from[..from.len() - 1];
        }

        if let Some(suffix) = new_path.strip_prefix(from) {
            if suffix.is_empty() || suffix == b"/" {
                // Exact match.
                new_path = to.to_vec();
                break;
            } else if suffix.starts_with(b"/") {
                // Proper prefix.
                new_path = concat_byte_slices(to, suffix);
                break;
            } else {
                // Not a real prefix (e.g. `abc` matches `abcd/ef`),
                // but it really is a different directory.
                continue;
            }
        }
    }

    let mut destination = None;

    // Find destination path.
    for (from, to) in path_mappings {
        let mut from = from.as_bytes();

        // If `from` ends with a `/`, ignore it. It simplifies further logic.
        // PERF: This avoids allocating a new `Vec` with `/` as suffix.
        if from.ends_with(b"/") {
            from = &from[..from.len() - 1];
        }

        if let Some(suffix) = new_path.strip_prefix(from) {
            if suffix.is_empty() || suffix == b"/" {
                // Exact match. This needs special treatment as the `tar` crate
                // skips empty file names (we can’t just unpack `.` in `to`).
                new_path = to.file_name().unwrap().as_bytes().to_vec();
                destination = to.parent().map(Path::to_path_buf);
                break;
            } else if suffix.starts_with(b"/") {
                // Proper prefix.
                new_path = suffix[1..].to_vec();
                destination = Some(PathBuf::clone(to));
                break;
            } else {
                // Not a real prefix (e.g. `abc` matches `abcd/ef`),
                // but it really is a different directory.
                continue;
            }
        }
    }

    if new_path != *original_path {
        if tracing::enabled!(tracing::Level::TRACE) {
            if let Some(ref destination) = destination {
                tracing::trace!(
                    "Mapping {:?} as {:?} in {:?}",
                    String::from_utf8_lossy(&original_path),
                    String::from_utf8_lossy(&new_path),
                    destination.display()
                );
            } else {
                tracing::trace!(
                    "Mapping {:?} to {:?}",
                    String::from_utf8_lossy(&original_path),
                    String::from_utf8_lossy(&new_path)
                );
            }
        }

        entry.set_path_bytes(new_path);
    }

    destination
}

/// A structure that holds the data necessary to revert all changes made
/// during a restoration when it is dropped. This ensures nothing has
/// changed if the restoration fails anywhere during the process.
#[derive(Debug, Default)]
pub struct RestoreRevertGuard {
    /// Destination paths which already existed, and which were backed up
    /// (e.g. `.bak`) to prevent data loss.
    ///
    /// This is a list of `(path, backup_path_opt)` pairs.
    paths: Vec<(PathBuf, Option<PathBuf>)>,

    /// Indicate if everything went successfully or not. If defused (which
    /// should be the case), dropping this will delete backed up paths. If not,
    /// It will delete created paths and recover backups.
    is_defused: bool,
}

impl RestoreRevertGuard {
    pub fn defuse(&mut self) {
        self.is_defused = true;
    }
}

impl Drop for RestoreRevertGuard {
    fn drop(&mut self) {
        if self.is_defused {
            for (_, backup_path_opt) in self.paths.iter() {
                if let Some(backup_path) = backup_path_opt {
                    if backup_path.exists() {
                        if let Err(err) = util::fs::remove(backup_path) {
                            tracing::error!("Could not delete path backup {backup_path:?}: {err:?}")
                        }
                    }
                }
            }
        } else {
            revert(self.paths.iter());
        }
    }
}

/// Backup destination paths to revert in case an error happens.
fn backup_destinations<'a>(
    path_mappings: impl Iterator<Item = &'a (OsString, PathBuf)>,
) -> Result<RestoreRevertGuard, RestorationError> {
    let mut revert_guard = RestoreRevertGuard::default();

    for (_, dst) in path_mappings {
        if dst.exists() {
            let Some(parent) = dst.parent() else {
                continue;
            };

            // If the destination directory can be backed up without copy,
            // do it. If not (e.g. directory is mounted), backup up children
            // individually.
            if is_same_device(dst, parent)
                // NOTE: If an error happens here, it aborts the backup
                //   restoration and reverts all changes made until then.
                .map_err(|err| RestorationError::PathBackupFailed {
                    path: PathBuf::clone(dst),
                    source: anyhow::Error::new(err).context("Failed testing device"),
                })?
            {
                let dst_bak = util::fs::backup_path(dst)
                    // NOTE: If an error happens here, it aborts the backup
                    //   restoration and reverts all changes made until then.
                    .map_err(|err| RestorationError::PathBackupFailed {
                        path: PathBuf::clone(dst),
                        source: anyhow::Error::new(err).context("Failed backing up dir"),
                    })?;

                (revert_guard.paths).push((PathBuf::clone(dst), Some(dst_bak)));
            } else {
                let fixme = "Remove unwraps";
                // NOTE: Read all children instead of iterating because we’ll
                //   be creating more children while iterating (potentially
                //   creating infinite loops).
                let children = std::fs::read_dir(dst).unwrap().collect::<Vec<_>>();

                for child in children {
                    let child = child.unwrap();

                    let child_path = &child.path();

                    let child_bak = util::fs::backup_path(child_path)
                        // NOTE: If an error happens here, it aborts the backup
                        //   restoration and reverts all changes made until then.
                        .map_err(|err| RestorationError::PathBackupFailed {
                            path: PathBuf::clone(child_path),
                            source: anyhow::Error::new(err).context("Failed backing up child"),
                        })?;

                    (revert_guard.paths).push((PathBuf::clone(child_path), Some(child_bak)));
                }
            }
        } else {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).context("Could not create restore destinations")?;
            }

            revert_guard.paths.push((PathBuf::clone(dst), None));
        }
    }

    Ok(revert_guard)
}

/// Note that this is best-effort, meaning we’re already doing error recovery
/// at this point so we can’t recover from subsequent internal errors.
#[cold]
fn revert<'a>(paths: impl Iterator<Item = &'a (PathBuf, Option<PathBuf>)>) {
    use std::fs;

    for (path, backup_path_opt) in paths {
        if path.exists() {
            if let Err(err) = util::fs::remove(path) {
                tracing::error!("Could not delete created path {path:?}: {err:?}")
            }
        }

        if let Some(backup_path) = backup_path_opt {
            if let Err(err) = fs::rename(&backup_path, &path) {
                tracing::error!("Could not recover {path:?}: {err:?}")
            };
        }
    }
}

// MARK: - Extraction (unarchiving)

#[derive(Debug, thiserror::Error)]
pub enum ExtractionError {
    #[error(transparent)]
    VerificationError(#[from] crate::verification::VerificationError),

    #[error("Invalid backup")]
    InvalidBackup(#[source] anyhow::Error),

    #[error("Unknown backup version: {0}.")]
    UnknownBackupVersion(u8),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<std::io::Error> for ExtractionError {
    fn from(error: std::io::Error) -> Self {
        Self::Other(anyhow::Error::from(error))
    }
}

struct RawReadStats<'a, H: RestoreBackupEventHandler> {
    backup_id: &'a BackupId,
    event_handler: &'a mut H,
}

impl<'a, H: RestoreBackupEventHandler> StreamStats for RawReadStats<'a, H> {
    fn record_chunk(&mut self, len: usize) {
        self.event_handler
            .on_restoration_progress(self.backup_id, len);
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
