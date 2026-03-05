// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use tempfile::TempDir;

use crate::{archiving::ArchiveBlueprint, util::safe_replace};

#[derive(Debug)]
pub struct RestorationSuccess;

pub(crate) fn restore(
    tmp_dir: TempDir,
    blueprint: &ArchiveBlueprint,
) -> Result<RestorationSuccess, anyhow::Error> {
    let fixme = "Do this atomically (all or nothing), by keeping backups.";

    for (dir_name, dst) in blueprint.paths.iter() {
        let src = tmp_dir.path().join(dir_name);

        safe_replace(src, dst)?;
    }

    Ok(RestorationSuccess)
}
