// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub struct NoopEventHandler;

impl crate::CreateBackupEventHandler for NoopEventHandler {}
impl crate::RestoreBackupEventHandler for NoopEventHandler {}

impl crate::archiving::ExtractBackupEventHandler for NoopEventHandler {}
