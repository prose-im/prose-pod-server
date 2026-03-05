# To Do

## Known issues (to be fixed)

High priority (ordered):

1. Make Pod Server restoration atomic.
1. Although the Pod Server restoration in itself is atomic, if the Pod API
   restoration fails we end up in an incorrect state. To prevent that we should
   let the Pod API try to restoration first and only then finish the Pod Server
   restoration.

Medium priority (unordered):

- Return backup size when getting details.

Low priority (unordered):

- Fallback to SHA-256 hash if signature from an unknown key. Use cases:
  - Old backup + PGP still configured but key lost.
  - PGP now configured, malicious actor plants signatures for old unsigned
    backups, rendering them unrestorable.

## Backlog

High priority (ordered):

1. Add S3 end-to-end integration tests.
1. Return download link.
1. Customizable backup name prefix (use case: Prose Cloud).

Medium priority (unordered):

- Overridable archiving blueprints (with non-overridable packaged ones).
- Cache S3 requests.
  - Also cache metadata when listing objects (it should already be present).
    - Or always return metadata directly; actually that would be better.
- Keep backup cached for some time after checking integrity.
  - One might open the details modal, then click “Restore”. It’s unfortunate we
    download the backup twice.
  - Keep a max size of backups cached, do not purge after some time.
    - If threshold would be passed when adding next backup, purge oldest cache
      entries until threshold is respected.
    - If backup size > threshold, remove backup after download. It might cause
      a second download soon after, but it’s what the operator asked for.
- Only allow one restore at a time.

Low priority (unordered):

- Add tests for all features and supported edge cases.
- Support dynamically adding encryption recipients without breaking checksum
  - Yep, that’s possible!
- Return all layer sizes when getting backup details (?).
- Return stats after restore (?).
- Stream stats when restoring.
