// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::fs;
use std::io;
use std::path::Path;

pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    fn inner(src: &Path, dst: &Path) -> io::Result<()> {
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if file_type.is_dir() {
                fs::create_dir_all(&dst_path)?;
                inner(&src_path, &dst_path)?;
            } else if file_type.is_file() {
                fs::copy(&src_path, &dst_path)?;
            } else {
                // NOTE: File can be a symlink.
                continue;
            }
        }

        Ok(())
    }

    let src = src.as_ref();
    let dst = dst.as_ref();

    if dst.exists() {
        if dst.read_dir().unwrap().count() > 1 {
            return Err(io::Error::other(format!("Directory {dst:?} not empty.")));
        }
    } else {
        fs::create_dir_all(dst)?;
    }

    inner(src, dst)
}

pub fn create_files<P: AsRef<Path>>(
    root: impl AsRef<Path>,
    files: impl IntoIterator<Item = P>,
) -> Result<(), anyhow::Error> {
    use anyhow::Context as _;

    fn mkdir(path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
        let path = path.as_ref();
        if !path.is_dir() {
            fs::create_dir_all(path).context(format!(
                "Failed creating dir at '{path}'",
                path = path.display()
            ))?;
        }
        Ok(())
    }
    fn touch(path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
        let path = path.as_ref();
        if !path.is_file() {
            fs::File::create(path).context(format!(
                "Failed creating file at '{path}'",
                path = path.display()
            ))?;
        }
        Ok(())
    }

    let root = root.as_ref();

    for path in files {
        let path = path.as_ref();

        if path.display().to_string().ends_with('/') {
            mkdir(root.join(path))?;
        } else {
            touch(root.join(path))?;
        }
    }

    Ok(())
}

// Map directories into test temporary directory, and create it.
#[cfg(feature = "storage-fs")]
pub fn map_storage_directories_in_test_dir(
    config_toml: &mut toml::Table,
    test_data_path: impl AsRef<std::path::Path>,
) -> Result<(), std::io::Error> {
    let test_data_path = test_data_path.as_ref();

    let storage = config_toml["storage"].as_table_mut().unwrap();
    if storage.contains_key("backups") {
        let backups_dir = &mut storage["backups"]["fs"]["directory"];
        let backups_store_path = test_data_path.join(backups_dir.as_str().unwrap());
        std::fs::create_dir_all(&backups_store_path)?;
        *backups_dir = toml::Value::String(backups_store_path.display().to_string());

        let checks_dir = &mut storage["checks"]["fs"]["directory"];
        let checks_store_path = test_data_path.join(checks_dir.as_str().unwrap());
        std::fs::create_dir_all(&checks_store_path)?;
        *checks_dir = toml::Value::String(checks_store_path.display().to_string());
    } else {
        let dir = &mut storage["fs"]["directory"];
        let store_path = test_data_path.join(dir.as_str().unwrap());
        std::fs::create_dir_all(&store_path)?;
        *dir = toml::Value::String(store_path.display().to_string());
    }

    let caching = config_toml
        .entry("caching")
        .or_insert(toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .unwrap();
    caching.entry("cache_dir").or_insert_with(|| {
        let cache_dir_path = test_data_path.join("cache");
        std::fs::create_dir_all(&cache_dir_path).unwrap();
        toml::Value::String(cache_dir_path.display().to_string())
    });

    Ok(())
}
