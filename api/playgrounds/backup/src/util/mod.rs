// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod serde;

/// Casting with `as` can yield incorrect values and similar issues
/// happen with `clamp`. This function ensures no overflow happens.
pub fn saturating_i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

/// Renames a file/directory (moves it to a new location), temporarily backing
/// it up and restoring it on failure (if applicable).
pub fn safe_replace<'a>(
    src: impl AsRef<std::path::Path>,
    dst: &'a std::path::Path,
) -> std::io::Result<Option<PathGuard>> {
    use std::fs;
    use std::path::PathBuf;

    let backup = if fs::exists(dst)? {
        let mut backup_path = dst.with_extension("bak");

        // If file already exists, switch to a unique name.
        if fs::exists(&backup_path)? {
            #[cold]
            fn use_unique_name(backup_path: &mut PathBuf, dst: &std::path::Path) {
                *backup_path = dst.with_extension(format!("{}.bak", unix_timestamp()));
            }
            use_unique_name(&mut backup_path, &dst)
        }

        fs::rename(dst, &backup_path)?;

        // WARN: We MUST create the `PathGuard` **after** running `fs::rename`.
        //   If we create it before and `fs::rename` ends up failing because the
        //   path already exists, `PathGuard::drop` would delete the wrong file!
        Some(PathGuard::new(backup_path))
    } else {
        None
    };

    if let Some(parent) = dst.parent() {
        if !fs::exists(parent)? {
            fs::create_dir_all(parent)?;
        }
    }

    match fs::rename(src, &dst) {
        Ok(()) => {
            // NOTE: Backup will automatically be cleaned up
            //   when `backup` is dropped.
            Ok(backup)
        }
        Err(err) => {
            // Restore backup on failure.
            if let Some(mut backup) = backup {
                fs::rename(&backup, &dst)?;

                // Defuse the `PathGuard` to avoid running its destructor.
                backup.defuse();
            }

            Err(err)
        }
    }
}

/// Deletes a path (file or directory) when dropped.
pub struct PathGuard {
    path: std::path::PathBuf,
    active: bool,
}

impl PathGuard {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path, active: true }
    }

    pub fn defuse(&mut self) {
        self.active = false;
    }
}

impl std::ops::Deref for PathGuard {
    type Target = std::path::PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<std::path::Path> for PathGuard {
    fn as_ref(&self) -> &std::path::Path {
        self.path.as_path()
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        let ref path = self.path;

        if matches!(path.try_exists(), Ok(false)) {
            return;
        }

        // Best-effort (cannot recover on cleanup).
        if path.is_dir() {
            std::fs::remove_dir_all(&path).unwrap_or_else(|err| {
                tracing::error!(
                    "Could not delete directory `{path}`: {err:?}",
                    path = path.display()
                )
            })
        } else {
            std::fs::remove_file(&path).unwrap_or_else(|err| {
                tracing::error!(
                    "Could not delete file `{path}`: {err:?}",
                    path = path.display()
                )
            })
        }
    }
}

pub trait SystemTimeExt {
    fn unix_timestamp(&self) -> u64;
}

impl SystemTimeExt for std::time::SystemTime {
    #[inline]
    fn unix_timestamp(&self) -> u64 {
        use std::time::{Duration, UNIX_EPOCH};

        self.duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs()
    }
}

#[inline(always)]
pub fn unix_timestamp() -> u64 {
    std::time::SystemTime::now().unix_timestamp()
}

/// Panic in debug mode.
///
/// To use with [`Result::inspect_err`].
#[inline(always)]
pub fn debug_panic<E: std::fmt::Debug>(error: &E) {
    if cfg!(debug_assertions) {
        panic!("{error:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_saturating_i64_to_u64() {
        // Casting with `as` can yield incorrect values:
        assert_eq!(i64::MIN, -9223372036854775808);
        assert_eq!(i64::MIN as u64, 9223372036854775808);

        assert_eq!(u64::MIN, 0);
        assert_eq!(saturating_i64_to_u64(i64::MIN), 0);
        assert_eq!(i64::MAX, 9223372036854775807);
        assert_eq!(saturating_i64_to_u64(i64::MAX), 9223372036854775807);
    }
}
