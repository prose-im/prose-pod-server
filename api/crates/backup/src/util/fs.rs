// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::util::unix_timestamp;

/// Deletes any path and its children.
#[inline]
pub fn remove(path: &std::path::Path) -> Result<(), std::io::Error> {
    use std::fs;

    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub fn backup_path(path: &std::path::Path) -> Result<std::path::PathBuf, std::io::Error> {
    use std::fs;
    use std::path::{Path, PathBuf};

    let mut backup_path = path.with_added_extension("bak");

    // If file already exists, switch to a unique name.
    if fs::exists(&backup_path)? {
        #[cold]
        fn use_unique_name(backup_path: &mut PathBuf, path: &Path) {
            *backup_path = path.with_added_extension(format!("{}.bak", unix_timestamp()));
        }
        use_unique_name(&mut backup_path, &path)
    }

    fs::rename(path, &backup_path)?;

    Ok(backup_path)
}

/// Deletes a path (file or directory) when dropped.
///
/// NOTE: To defuse, use `std::mem::take(&mut self.path)`.
pub struct PathGuard {
    path: std::path::PathBuf,
}

impl PathGuard {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }
}

impl std::ops::Deref for PathGuard {
    type Target = std::path::PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl std::fmt::Debug for PathGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.path, f)
    }
}

impl AsRef<std::path::Path> for PathGuard {
    fn as_ref(&self) -> &std::path::Path {
        self.path.as_path()
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        let path = &self.path;

        if matches!(path.try_exists(), Ok(false)) {
            return;
        }

        #[cfg(debug_assertions)]
        tracing::trace!("[Drop] Deleting `{path}`…", path = path.display());

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

/// Tells whether or not both paths are on the same device.
///
/// This is useful to detect if a file could be renamed.
pub fn is_same_device(
    p1: impl AsRef<std::path::Path>,
    p2: impl AsRef<std::path::Path>,
) -> Result<bool, std::io::Error> {
    use std::os::unix::fs::MetadataExt as _;

    let m1 = std::fs::metadata(p1)?;
    let m2 = std::fs::metadata(p2)?;

    Ok(m1.dev() == m2.dev())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, ffi::OsString, fs};

    use super::*;

    /// Tests that `backup_path` doesn’t fail if a `*.bak` file already exists.
    #[test]
    fn test_backup_path_existing_bak() {
        let tmpdir = tempfile::TempDir::with_prefix(std::env!("CARGO_CRATE_NAME")).unwrap();
        let tmp = tmpdir.path();

        let src = tmp.join("src");
        let dst = tmp.join("dst");
        let dst_bak = dst.with_added_extension("bak");

        fs::create_dir(&src).unwrap();
        fs::create_dir(&dst).unwrap();
        fs::create_dir(&dst_bak).unwrap();
        // NOTE: We need to create a file, because `fs::rename` doesn’t fail
        //   when renaming to an existing, but empty, directory.
        fs::write(dst_bak.join("foo"), "").unwrap();

        #[rustfmt::skip]
        assert_dirs!(tmp, ["src", "dst", "dst.bak"]);

        let new_path = backup_path(&src).unwrap();
        let new_name = new_path.file_name().unwrap().to_os_string();
        assert_ne!(new_name, "dst");

        assert_dirs!(
            tmp,
            [
                OsString::from("dst"),
                OsString::from("dst.bak"),
                new_name,
            ]
        );
    }

    // /// Tests that `safe_replace` doesn’t fail if the destination is missing.
    // #[test]
    // fn test_safe_replace_missing_dst() {
    //     let tmpdir = tempfile::TempDir::with_prefix(std::env!("CARGO_CRATE_NAME")).unwrap();
    //     let tmp = tmpdir.path();

    //     let src = tmp.join("src");
    //     let dst = tmp.join("dst");

    //     fs::create_dir(&src).unwrap();

    //     assert_dirs!(tmp, ["src"]);

    //     let backup_opt = safe_replace(src, &dst).unwrap();
    //     assert!(matches!(backup_opt, None));

    //     assert_dirs!(tmp, ["dst"]);
    // }

    // /// Tests that `safe_replace` doesn’t fail if the destination parent
    // /// directory is missing.
    // #[test]
    // fn test_safe_replace_missing_dst_parent() {
    //     let tmpdir = tempfile::TempDir::with_prefix(std::env!("CARGO_CRATE_NAME")).unwrap();
    //     let tmp = tmpdir.path();

    //     let src = tmp.join("src");
    //     let dst = tmp.join("path/to/dst");

    //     fs::create_dir(&src).unwrap();

    //     assert_dirs!(tmp, ["src"]);

    //     let backup_opt = safe_replace(src, &dst).unwrap();
    //     assert!(matches!(backup_opt, None));

    //     assert_dirs!(tmp, ["path"]);
    // }

    // /// Tests that `safe_replace` doesn’t delete the directory backup if the
    // /// final move fails.
    // #[test]
    // fn test_safe_replace_revert_on_move_error() {
    //     let tmpdir = tempfile::TempDir::with_prefix(std::env!("CARGO_CRATE_NAME")).unwrap();
    //     let tmp = tmpdir.path();

    //     let src = tmp.join("foo/src");
    //     let dst = tmp.join("dst");

    //     fs::create_dir_all(&src).unwrap();
    //     fs::create_dir(&dst).unwrap();
    //     fs::set_permissions(tmp.join("foo"), Permissions::from_mode(0o500)).unwrap();

    //     assert_dirs!(tmp, ["foo", "dst"]);

    //     let res = safe_replace(src, &dst);
    //     assert!(res.is_err());
    //     let err = format!("{err:#}", err = anyhow::Error::from(res.err().unwrap()));
    //     assert_eq!(err, "Permission denied (os error 13)");

    //     assert_dirs!(tmp, ["foo", "dst"]);
    // }

    macro_rules! assert_dirs {
        ($tmp:expr, $expected:expr $(,)?) => {{
            let dirs: HashSet<OsString> = fs::read_dir($tmp)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            let expected = HashSet::from_iter($expected.into_iter().map(Into::into));
            assert_eq!(dirs, expected);
        }};
    }
    pub(self) use assert_dirs;
}
