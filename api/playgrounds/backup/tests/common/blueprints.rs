// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use prose_backup::archiving::ArchiveBlueprint;

use crate::common::prelude::*;

pub const BLUEPRINT_LOCAL_DATA: u8 = 1;
pub const BLUEPRINT_POD_API_DEMO: u8 = 2;

#[rustfmt::skip]
pub fn test_blueprints() -> HashMap<u8, ArchiveBlueprint> {
    BlueprintsBuilder::new()
        .insert_paths(
            BLUEPRINT_LOCAL_DATA,
            [
                ("foo-data", "foo"),
                ("bar-data", "bar"),
                ("baz-data", "baz"),
            ]
            .into_iter(),
        )
        .insert_paths(
            BLUEPRINT_POD_API_DEMO,
            [
                ("prosody-data", "prosody/data"),
                ("prosody-config", "prosody/config"),
                ("prose-pod-server-data", "prose-pod-server-data"),
            ]
            .into_iter(),
        )
        .build()
}

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

    pub fn insert_paths<Dst, Src, I>(mut self, version: u8, paths: I) -> Self
    where
        I: Iterator<Item = (Dst, Src)>,
        Dst: ToString,
        Src: AsRef<Path>,
    {
        self.res.insert(version, ArchiveBlueprint::from_iter(paths));
        self
    }

    pub fn build(self) -> HashMap<u8, ArchiveBlueprint> {
        self.res
    }
}
