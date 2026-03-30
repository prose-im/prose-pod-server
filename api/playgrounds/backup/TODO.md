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

1. Support custom object prefix (use case: S3 directory-style prefix).
1. Ensure the API works if backups and checks are in the same bucket.
1. Crash if same bucket + prefix is used for backups and checks.
   - Or support this use case.
     - Then provide a convenience configuration key to avoid needing to
       configure both `storage.backups` and `storage.checks`.

Medium priority (unordered):

- Make sure one can change the hashing algorithm to a non-default one but
  existing backups still restore, and vice-versa.
- Overridable archiving blueprints (with non-overridable packaged ones).
- Cache S3 requests.
  - Also cache metadata when listing objects (it should already be present).
    - Or always return metadata directly; actually that would be better.
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
- Stream stats when creating a backup.
- Recover a recently deleted backup.
  - Do not delete objects directly, but rather create a delete marker and truly
    delete only after some (configurable) time.
- Add more storage providers.
  - Take inspiration from <https://crates.io/crates/zesty-backup>.
