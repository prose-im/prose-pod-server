---
date: 2026-04-17T18:55:00+02:00
modified: 2026-05-12T21:14:00+02:00
authors:
  - Rémi Bardon <remi@remibardon.name>
---

# Restoration process

## High level

Backup restoration happens in two high level steps: first the backup is
downloaded and cached, then it is extracted and restored. Unarchiving
(a.k.a. archive extraction) can be a dangerous operation, which is why we
first validate the integrity (and authenticity, if possible) of the backup
and only then we proceed to unarchiving (instead of doing everything in a
single stream).

Keeping the backup in a local cache also reduces the number of calls to the
backup storage, which, in addition to being slow, are often financially
expensive. If a user checks the backup details multiple times, or checks the
details before deciding to restore, we can avoid redundant network calls.

In the early days of the library, we used to download, validate and extract
the backup whenever a user asked for details. It allowed processing the archive
a single time, but we later realized it prevented atomic operations when
destination paths were mounted (which is always the case in containers).

For reasons explained in [“Migration process”](./migration.md), the library now
extracts entries only when restoring a backup. This has the added benefit of
making validation faster.
