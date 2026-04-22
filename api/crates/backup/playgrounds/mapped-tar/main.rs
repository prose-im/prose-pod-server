//! Tests mapping paths while extracting an archive.

use std::fs::{self, File, Permissions};
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

#[rustfmt::skip]
mod blueprints {
    pub(crate) const V1: [(&str, &str); 2] = [
        ("foo-data", "var/lib/foo/"),
        ("bar-data", "var/lib/bar/"),
    ];
    pub(crate) const V2: [(&str, &str); 1] = [
        ("foo-data", "var/lib/foo/"),
    ];
    pub(crate) const V3: [(&str, &str); 1] = V2;
}
#[rustfmt::skip]
mod migrations {
    pub(crate) const V1_TO_V2: [(&str, &str); 1] = [
        ("bar-data/", "foo-data/bar/"),
    ];
    pub(crate) const V2_TO_V3: [(&str, &str); 1] = [
        ("foo-data/bar/b", "foo-data/c"),
    ];
}

#[test]
fn extract_as_v1() {
    let tmp_dir = TempDir::with_prefix(concat!(env!("CARGO_CRATE_NAME"), "-")).unwrap();
    let tmp_path = tmp_dir.path();

    let mut archive = new_archive_v1();

    let fs_root = TempDir::with_prefix_in("fs-root-", tmp_path).unwrap();
    let fs_root = fs_root.path();

    extract(&mut archive, fs_root, blueprints::V1, []).unwrap();

    #[rustfmt::skip]
    assert_eq!(
        sorted(tree(fs_root)),
        sorted([
            "var", "var/lib",
            "var/lib/foo", "var/lib/foo/a", "var/lib/foo/b",
            "var/lib/bar", "var/lib/bar/a", "var/lib/bar/b",
        ].into_iter().map(PathBuf::from).collect::<Vec<_>>())
    );

    assert_permissions!(fs_root.join("var/lib/bar/b"), 0o100640);
    assert_permissions!(fs_root.join("var/lib/bar/"), 0o40750);
}

#[test]
fn extract_as_v2() {
    let tmp_dir = TempDir::with_prefix(concat!(env!("CARGO_CRATE_NAME"), "-")).unwrap();
    let tmp_path = tmp_dir.path();

    let mut archive = new_archive_v1();

    let fs_root = TempDir::with_prefix_in("fs-root-", tmp_path).unwrap();
    let fs_root = fs_root.path();

    extract(&mut archive, fs_root, blueprints::V2, migrations::V1_TO_V2).unwrap();

    #[rustfmt::skip]
    assert_eq!(
        sorted(tree(fs_root)),
        sorted([
            "var", "var/lib",
            "var/lib/foo", "var/lib/foo/a", "var/lib/foo/b",
            "var/lib/foo/bar", "var/lib/foo/bar/a", "var/lib/foo/bar/b",
        ].into_iter().map(PathBuf::from).collect::<Vec<_>>())
    );

    assert_permissions!(fs_root.join("var/lib/foo/bar/b"), 0o100640);
    assert_permissions!(fs_root.join("var/lib/foo/bar/"), 0o40750);
}

#[test]
fn extract_as_v3() {
    let tmp_dir = TempDir::with_prefix(concat!(env!("CARGO_CRATE_NAME"), "-")).unwrap();
    let tmp_path = tmp_dir.path();

    let mut archive = new_archive_v1();

    let fs_root = TempDir::with_prefix_in("fs-root-", tmp_path).unwrap();
    let fs_root = fs_root.path();

    extract(
        &mut archive,
        fs_root,
        blueprints::V3,
        migrations::V1_TO_V2.into_iter().chain(migrations::V2_TO_V3),
    )
    .unwrap();

    #[rustfmt::skip]
    assert_eq!(
        sorted(tree(fs_root)),
        sorted([
            "var", "var/lib",
            "var/lib/foo", "var/lib/foo/a", "var/lib/foo/b", "var/lib/foo/c",
            "var/lib/foo/bar", "var/lib/foo/bar/a",
        ].into_iter().map(PathBuf::from).collect::<Vec<_>>())
    );

    assert_permissions!(fs_root.join("var/lib/foo/c"), 0o100640);
    assert_permissions!(fs_root.join("var/lib/foo/bar/"), 0o40750);
}

fn main() {
    println!("OK");
}

fn new_archive_v1() -> tar::Archive<std::io::Cursor<Vec<u8>>> {
    let tmp_dir = TempDir::with_prefix("tar-extract-mapped-").unwrap();
    let tmp_path = tmp_dir.path();

    let fs_root = tmp_path.join("fs-root");

    for path in [
        "var/lib/foo/",
        "var/lib/foo/a",
        "var/lib/foo/b",
        "var/lib/bar/",
        "var/lib/bar/a",
        "var/lib/bar/b",
    ] {
        if path.ends_with("/") {
            fs::create_dir_all(fs_root.join(path)).unwrap();
        } else {
            let mut file = File::create_new(fs_root.join(path)).unwrap();
            file.write_all(b"foo").unwrap();
        }
    }

    change_permissions(fs_root.join("var/lib/bar/b"), 0o100644, 0o100640);
    change_permissions(fs_root.join("var/lib/bar/"), 0o40755, 0o40750);

    let archive_bytes = archive(&fs_root, blueprints::V1);

    tar::Archive::new(std::io::Cursor::new(archive_bytes))
}

fn archive<'a>(fs_root: &Path, blueprint: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<u8> {
    let mut archive_bytes: Vec<u8> = Vec::new();

    let mut builder = tar::Builder::new(&mut archive_bytes);

    for (path, src_path) in blueprint.into_iter() {
        builder
            .append_dir_all(path, fs_root.join(src_path))
            .unwrap();
    }

    builder.finish().unwrap();
    drop(builder);

    archive_bytes
}

#[allow(dead_code)]
fn extract_custom<'a, R: std::io::Read + std::io::Seek>(
    archive: &mut tar::Archive<R>,
    dst: &Path,
    blueprint: impl IntoIterator<Item = (&'a str, &'a str)>,
    migrations: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> anyhow::Result<()> {
    let mappings = flatten(blueprint.into_iter(), migrations.into_iter());

    unpack(archive, dst, mappings)
}

fn extract<'a, R: std::io::Read>(
    archive: &mut tar::Archive<R>,
    dst: &Path,
    blueprint: impl IntoIterator<Item = (&'a str, &'a str)>,
    migrations: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> std::io::Result<()> {
    let mappings = flatten(blueprint.into_iter(), migrations.into_iter());

    archive.unpack_mapped(dst, |entry| {
        map_path(entry, mappings.iter());
    })
}

fn concat(a: &str, b: &str) -> String {
    let mut res = String::with_capacity(a.len() + b.len());
    res.push_str(a);
    res.push_str(b);
    res
}

fn flatten<'a>(
    blueprint: impl Iterator<Item = (&'a str, &'a str)>,
    migrations: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<(Box<str>, Box<str>)> {
    let mut mappings: Vec<(Box<str>, Box<str>)> =
        Vec::with_capacity((migrations.size_hint().0).saturating_add(blueprint.size_hint().0));

    // Apply migrations.
    for (path, src_path) in migrations {
        let mut path: Box<str> = Box::from(path);

        for (from, to) in mappings.iter() {
            if to.ends_with("/") {
                if let Some(suffix) = path.strip_prefix(to.as_ref()) {
                    let new_path = concat(from, suffix);
                    debug!("Mapped {path:?} to {new_path:?}");
                    path = Box::from(new_path);
                }
            } else {
                if path == *to {
                    debug!("Mapped {path:?} to {from:?}");
                    // PERF: No need to use `Rc` to make this clone cheaper, it
                    //   should happen very rarely. The extra allocations of
                    //   `Rc` would outweight the gains.
                    path = Box::clone(from);
                }
            }
        }

        mappings.push((path, Box::from(src_path)));
    }

    // Store mappings count to avoid iterating over newly added mappings as
    // they should not be mappable (optimization).
    let mappings_count = mappings.len();

    // Map volume names.
    for (path, src_path) in blueprint {
        assert!(!path.ends_with("/"));

        if src_path.ends_with("/") {
            let path: Box<str> = Box::from(concat(path, "/"));

            for (_, to) in mappings.iter_mut().take(mappings_count) {
                if let Some(suffix) = to.strip_prefix(path.as_ref()) {
                    let res = concat(src_path, suffix);
                    debug!("Mapped {to:?} to {res:?}");
                    *to = Box::from(res);
                }
            }

            mappings.push((path, Box::from(src_path)));
        } else {
            mappings.push((Box::from(path), Box::from(src_path)));
        }
    }

    // Sort mappings so the longer paths are first.
    mappings.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()));

    debug!("Mappings: {:#?}", fmt::AsMap(&mappings));

    mappings
}

fn map_path<'a, R: std::io::Read>(
    entry: &mut tar::Entry<R>,
    mappings: impl Iterator<Item = &'a (Box<str>, Box<str>)>,
) {
    let path = entry.path_bytes();
    for (from, to) in mappings {
        let from = from.as_bytes();
        let to = to.as_bytes();

        if from.ends_with(b"/") {
            assert!(to.ends_with(b"/"));

            if let Some(suffix) = path.strip_prefix(from) {
                let mut res: Vec<u8> = Vec::with_capacity(to.len() + 1 + suffix.len());
                res.extend_from_slice(to);
                res.extend_from_slice(suffix);
                debug!(
                    "Unpacking {:?} in {:?}",
                    String::from_utf8_lossy(&path),
                    String::from_utf8_lossy(&res)
                );
                entry.set_path_bytes(res);
                break;
            }
        } else {
            if path.as_ref() == from {
                let res = to.to_vec();
                debug!(
                    "Unpacking {:?} in {:?}",
                    String::from_utf8_lossy(&path),
                    String::from_utf8_lossy(&res)
                );
                entry.set_path_bytes(res);
                break;
            }
        }
    }
}

/// This is like [`tar::Archive::unpack`], but it maps paths during extraction.
fn unpack<R: std::io::Read + std::io::Seek>(
    archive: &mut tar::Archive<R>,
    dst: &Path,
    mappings: Vec<(Box<str>, Box<str>)>,
) -> anyhow::Result<()> {
    if dst.symlink_metadata().is_err() {
        fs::create_dir_all(dst).map_err(|e| {
            anyhow::Error::new(e).context(format!("failed to create `{}`", dst.display()))
        })?;
    }

    // Canonicalizing the dst directory will prepend the path with '\\?\'
    // on windows which will allow windows APIs to treat the path as an
    // extended-length path with a 32,767 character limit. Otherwise all
    // unpacked paths over 260 characters will fail on creation with a
    // NotFound exception.
    let dst = &dst.canonicalize().unwrap_or(dst.to_path_buf());

    // Delay any directory entries until the end (they will be created if needed by
    // descendants), to ensure that directory permissions do not interfere with descendant
    // extraction.
    let mut directories = Vec::new();
    for entry in archive.entries_with_seek()? {
        let mut file =
            entry.map_err(|e| anyhow::Error::new(e).context("failed to iterate over archive"))?;

        map_path(&mut file, mappings.iter());

        if file.header().entry_type() == tar::EntryType::Directory {
            directories.push(file);
        } else {
            file.unpack_in(dst)?;
        }
    }

    // Apply the directories.
    //
    // Note: the order of application is important to permissions. That is, we must traverse
    // the filesystem graph in topological ordering or else we risk not being able to create
    // child directories within those of more restrictive permissions. See [0] for details.
    //
    // [0]: <https://github.com/alexcrichton/tar-rs/issues/242>
    directories.sort_by(|a, b| b.path_bytes().cmp(&a.path_bytes()));
    for mut dir in directories {
        dir.unpack_in(dst)?;
    }

    Ok(())
}

fn sorted(mut vec: Vec<PathBuf>) -> Vec<PathBuf> {
    vec.sort();
    vec
}

fn change_permissions(path: impl AsRef<Path>, expected_mode: u32, new_mode: u32) {
    let path = path.as_ref();

    assert_permissions!(path, expected_mode);

    fs::set_permissions(path, Permissions::from_mode(new_mode)).unwrap();
}

macro_rules! assert_permissions {
    ($path:expr, $expected:expr) => {{
        let mode = fs::metadata($path).unwrap().permissions().mode();
        let expected_mode = $expected;
        assert_eq!(
            mode,
            expected_mode,
            "{path}: {mode:#o} != {expected_mode:#o}",
            path = $path.display()
        );
    }};
}
pub(crate) use assert_permissions;

macro_rules! debug {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        eprintln!($($arg)*);
    };
}
pub(crate) use debug;

fn tree(path: impl AsRef<Path>) -> Vec<PathBuf> {
    fn subtree(tree: &mut Vec<PathBuf>, dir_path: impl AsRef<Path>, base: impl AsRef<Path>) {
        let base = base.as_ref();

        for entry in fs::read_dir(dir_path).unwrap() {
            let entry = entry.unwrap();

            tree.push(entry.path().strip_prefix(base).unwrap().to_path_buf());

            if entry.metadata().unwrap().is_dir() {
                subtree(tree, entry.path(), base);
            }
        }
    }

    let dir_path = path.as_ref();

    let mut res = Vec::new();

    subtree(&mut res, dir_path, dir_path);

    res
}

mod fmt {
    use std::fmt;

    pub struct AsMap<'a, K, V>(pub &'a [(K, V)]);

    impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for AsMap<'_, K, V> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let mut map = f.debug_map();
            for (k, v) in self.0 {
                map.entry(k, v);
            }
            map.finish()
        }
    }
}
