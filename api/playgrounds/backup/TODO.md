# To Do

## Known issues (to be fixed)

High priority (ordered):

None.

Medium priority (unordered):

- Backups remain partially uploaded if an integrity check upload fails.
- Passphrase-protected OpenPGP secret key material are not supported.

Low priority (unordered):

- Fallback to SHA-256 hash if signature from an unknown key. Use cases:
  - Old backup + PGP still configured but key lost.
  - PGP now configured, malicious actor plants signatures for old unsigned
    backups, rendering them unrestorable.

## Backlog (planned)

High priority (ordered):

1. Test atomicity of restores.

Medium priority (unordered):

- Overridable archiving blueprints (with non-overridable packaged ones).
- Cache S3 requests.
  - Also cache metadata when listing objects (it should already be present).
    - Or always return metadata directly; actually that would be better.
- Only allow one restore at a time.
- Type errors more granularly.
- Test that permissions are correctly kept when restoring a backup.

Low priority (unordered):

- Add tests for all features and supported edge cases.
- Use `tempfile::tempfile` instead of `tempfile::tempdir` to save backups.

## Feature ideas (unordered)

- Support dynamically adding encryption recipients without breaking checksum
  - Yep, that’s possible!
- Return all layer sizes when getting backup details (?).
- Return stats after restore (?).
- Stream stats when restoring.
- Recover a recently deleted backup.
  - Do not delete objects directly, but rather create a delete marker and truly
    delete only after some (configurable) time.
- Add more storage providers.
  - Take inspiration from <https://crates.io/crates/zesty-backup>.
