---
date: 2026-04-18T17:09:00+02:00
modified: 2026-05-12T21:56:00+02:00
authors:
  - Rémi Bardon <remi@remibardon.name>
---

# Migration process

Both the internal structure of backups and the extraction paths can evolve over
time. Because we wouldn’t want changes to render previous backups unrestorable,
we had to come up with a migration process. This document explains our thought
process and the architecture we went for.

## The naive approach: extracting first, then renaming

At first, the API used to extract the archive in one directory in a first step,
then run migrations inside (i.e. move files around) and finally rename
directories to their final destination in an atomic fashion[^safe-replace].
This process was very simple to reason about, kept directories clean as long as
possible, and worked perfectly in tests.

However, when deployed in a real production-ready app, it failed because
renames don’t work across devices (i.e. mounted paths). This meant that the
library was useless in containerized environments, but also in any other
environment where some paths are mounted from remote storage.

[^safe-replace]: If the destination already existed, it was backed up, and if one rename failed then everything was reverted.

## Requirements

While looking for a new process, we laid down the requirements:

- Backups MUST stay restorable even if it needs multiple migrations.
- Migrations MUST NOT create noticeable overhead when restoring a backup.
  With `n` entries in the archive, restoration should still be `O(n)`.
- Backups MUST be restorable even if some “volumes” are on another device
  (e.g. mounted path).
- <details>
    <summary>Migration blueprints MUST support using “volume” names (open details for an explanation).</summary>

    For example, this should work (note the migration not using full
    destination paths):

    ```
    Backup version 1: [
      "foo-data" -> "/var/lib/foo",
      "bar-data" -> "/var/lib/bar",
    ]
    Backup version 2: [
      "foo-data" -> "/var/lib/foo",
    ]

    Migration 1 to 2: [
      "bar-data" -> "foo-data/bar",
    ]
    ```
  </details>

All of this, while keeping throughput as high as possible.

## The obvious (but bad) approach: copying files

Since one can’t move `/tmp/my_backup/foo-data` to `/var/lib/data` if
`/var/lib/data` is mounted, one solution would be to copy everything from
`/tmp/my_backup/foo-data` to `/var/lib/data`. It would work, but:

- It would make atomic restoration a lot harder to implement (think about
  backing up existing contents and reverting all operations in case of failure).
- It would require a lot of custom backup/copy/delete/restore logic, which
  would be error prone (and we really don’t want errors in a backup system).
- And the elephant in the room: it would be subpar in terms of performance.
  The amount of storage needed would be doubled[^tripled] for some time
  (preventing use in constrained environments), execution time would increase a
  lot (I/O operations are computationally expensive) and other similar issues.

[^tripled]: Even tripled if we consider the data backed up during restoration!

## The less obvious (but still bad) approach: using symbolic links

One possible solution would be to use symbolic links in a temporary directory
to “redirect” writes to other devices. While it is theoretically a great idea,
it’s not technically acceptable:

- It doesn’t look like it, but one cannot write files inside a symbolically
  linked directory. Paths have to be “resolved” first, via a process called
  “canonicalization”. This process consists of walking through every path
  component, checking whether it is a symbolic link and replacing the path
  up to it if so. It means that for `n` path components, one must make `n`
  I/O calls to query the file metadata, interpret it (introducing branching),
  and allocate memory every time a symbolic link is found. That’s a lot, and
  we’d have to do that for each archive entry (there could be thousands!).
  The performance hit would be immense.
- For safety reasons, the `tar` crate ensures that an entry cannot be unpacked
  outside of its parent directory[^inside-dst].
- Nested symbolic links would have to be handled differently, otherwise writes
  would fail.

[^inside-dst]: Ironically, `validate_inside_dst` extensively uses canonicalization. It doesn’t mean we should do one more canonicalization, but means we should work on improving the `tar` crate so we can skip canonicalization for tructed paths. This would improve the performance of the crate, which could benefit from improvements.

## The smarter approach: migrating paths during extraction

What we want is to extract the files on the correct device while processing the
archive, to avoid any cross-device copy. One way to do that could be to change
where archive entries are extracted by mapping their paths using migration
blueprints. Unfortunately, tar archives store each file and directory as a
separate entry —one cannot just rename a directory.

Because backups can be very large, we do everything in a streaming fashion.
This means we’d have to map every single file path by applying relevant
migrations. For `n` archive entries and `m` migrated paths, this solution would
mean comparing string prefixes `m` times, then constructing a new string up to
`m` times, for each of the `n` entries. Imagine `n=10000` and `m=100`, it would
involve a huge amount of memory allocations (up to `n*m`).

We want a solution in which both extraction and migration are at most linear,
without compounding (i.e. `O(n+m)`), but we can’t have it with this process,
so we went for an in-between. Before extracting, the library “flattens” all the
migration paths and sorts them by descending length, ensuring we can stop at
the very first match. It’s not exactly `O(n+m)`, but it’s better than `O(n*m)`.

The library also avoids string comparisons by working with byte slices directly
(thanks to [this addition in the `tar` crate][tar#448]) and tries to avoid
unnecessary allocations. For more information, see `restoration.rs`.

[tar#448]: https://github.com/alexcrichton/tar-rs/pull/448 "Add `Entry::set_path_bytes` (Pull Request #448) by RemiBardon on alexcrichton/tar-rs"
