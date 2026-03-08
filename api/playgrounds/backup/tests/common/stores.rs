// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub fn fs_store(
    path: impl AsRef<std::path::Path>,
) -> Result<prose_backup::stores::Fs, std::io::Error> {
    let path = path.as_ref();

    std::fs::create_dir_all(path)?;

    Ok(prose_backup::stores::Fs::builder()
        .overwrite(true)
        .directory(path)
        .build())
}
