# To Do

## Known issues (to be fixed)

High priority (ordered):

None.

Medium priority (unordered):

None.

Low priority (unordered):

- Fallback to SHA-256 hash if signature from an unknown key. Use cases:
  - Old backup + PGP still configured but key lost.
  - PGP now configured, malicious actor plants signatures for old unsigned
    backups, rendering them unrestorable.

## Backlog

High priority (ordered):

None.

Medium priority (unordered):

- Overridable archiving blueprints (with non-overridable packaged ones).
- Cache S3 requests.
  - Also cache metadata when listing objects (it should already be present).
    - Or always return metadata directly; actually that would be better.
- Only allow one restore at a time.
- Type errors more granularly.

Low priority (unordered):

- Add tests for all features and supported edge cases.

## Feature ideas (unordered)

- Support dynamically adding encryption recipients without breaking checksum
  - Yep, that’s possible!
- Return all layer sizes when getting backup details (?).
- Return stats after restore (?).
- Stream stats when restoring.
- Recover a recently deleted backup.
  - Do not delete objects directly, but rather create a delete marker and truly
    delete only after some (configurable) time.
