# To Do

## Known issues (to be fixed)

High priority (ordered):

None.

Medium priority (unordered):

- Backups remain partially uploaded if an integrity check upload fails.
- Passphrase-protected OpenPGP secret key material are not supported.

Low priority (unordered):

None.

## Backlog (planned)

High priority (ordered):

None.

Medium priority (unordered):

- Overridable archiving blueprints (with non-overridable packaged ones).
- Cache S3 requests.
  - Also cache metadata when listing objects (it should already be present).
    - Or always return metadata directly; actually that would be better.
- Type errors more granularly.

Low priority (unordered):

- Add tests for all features and supported edge cases.

## Feature ideas (unordered)

- Support dynamically adding encryption recipients without breaking checksum
  - Yep, that’s possible!
- Return all layer sizes (raw, decrypted, decompressed…) when getting backup
  details (?).
- Return archive contents tree when getting backup details (?).
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
