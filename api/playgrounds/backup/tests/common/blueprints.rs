// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{collections::HashMap, path::Path};

use prose_backup::archiving::ArchiveBlueprint;

pub const BLUEPRINT_PATHS_POD_API_DEMO: [(&str, &str); 3] = [
    ("prosody-data", "prosody/data"),
    ("prosody-config", "prosody/config"),
    ("prose-pod-server-data", "prose-pod-server-data"),
];

pub trait ArchiveBlueprintExt {
    fn src_relative_to(&self, origin: impl AsRef<Path>) -> Self;
}

impl ArchiveBlueprintExt for ArchiveBlueprint {
    fn src_relative_to(&self, origin: impl AsRef<Path>) -> Self {
        Self::from_iter(
            self.paths
                .iter()
                .map(|(dst, src)| (dst.to_owned(), origin.as_ref().join(src))),
        )
    }
}

pub struct BlueprintsBuilder {
    res: HashMap<u8, ArchiveBlueprint>,
}

impl BlueprintsBuilder {
    pub fn new() -> Self {
        Self {
            res: HashMap::new(),
        }
    }

    pub fn insert(mut self, version: u8, blueprint: ArchiveBlueprint) -> Self {
        self.res.insert(version, blueprint);
        self
    }

    pub fn build(self) -> HashMap<u8, ArchiveBlueprint> {
        self.res
    }
}
