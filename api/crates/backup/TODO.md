# To Do

## Known issues (to be fixed)

High priority (ordered):

None.

Medium priority (unordered):

None.

Low priority (unordered):

- If compression, encryption and signing are all disabled, the progress when
  restoring doesn’t reach 100% (`tar` skips the last block). Fix this, but also
  make sure integrity is checked (no issue when signing is enabled).
- When restoration paths are mounted, children are moved individually and the
  directory’s permissions are not updated.

## Backlog (planned)

High priority (ordered):

- Test passing passhprases via configuration (not tested IIRC).
  - Fingerptint `.to_string()` might contain spaces and break lookup.

Medium priority (unordered):

- Rework the progress calculation (avoid archive size estimation).
  - Total file size cannot be estimated if additional data is an archive with
    PAX headers (when restoring).
- Support batch deletion.
- Overridable archiving blueprints (keeping packaged ones non-overridable).
- Type errors more granularly.
- Test backup progress with long paths (GNU LongLink).
  - Archive size estimation might be wrong.
- Cache S3 requests.
  - Also cache metadata when listing objects (it should already be present).
    - Or always return metadata directly; actually that would be better.

Low priority (unordered):

- Add tests for all features and supported edge cases.
- Read magic bytes instead of checking object extensions.
- Support cleanup of expired markers when using S3 Object Lock.

## Feature ideas (unordered)

- Support dynamically adding encryption recipients without breaking checksum
  - Yep, that’s possible!
- Return all layer sizes (raw, decrypted, decompressed…) when getting backup
  details (?).
- (A) Return backup version in backup details.
- Return archive contents tree when getting backup details (?).
  - Requires (A).
  - Iterate on entries but DO NOT EXTRACT.
  - Return file sizes.
  - Maybe not full FS tree but a certain level deep (e.g. 2).
    - Aggregate children sizes if incomplete tree.
    - Tree depth parameterized at the HTTP API level?
      - If not, in the static config at least.
  - Maybe map entry keys to destination paths.
- Return stats after restore (?).
- Stream stats when restoring.
- Recover a recently deleted backup.
  - Do not delete objects directly, but rather create a delete marker and truly
    delete only after some (configurable) time.
- Add more storage providers.
  - Take inspiration from <https://crates.io/crates/zesty-backup>.
