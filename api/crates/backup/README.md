# Backup & Restore

A simple yet powerful library for creating and restoring backups, tailored for
[Prose]’s needs.

## Features

- Create, list and delete backups
- Backups have a human-readable description
- Backups are compressed using [Zstandard] (extremely fast, high compression)
- Backups can be encrypted (using your own OpenPGP key)
- Backups can be signed (using your own OpenPGP key)
- Backup structures can evolve (while keeping old backups restorable)
- OpenPGP keys can be rotated (while keeping old backups restorable)
- Minimal configuration (sensible defaults)
- Minimal network overhead (no duplicate or unnecessary requests)
- Small memory footprint (everything is streamed)
- Backup and restore operations can have progress indicators
- Large files (e.g. 1GB) are supported

Advanced features:

- Fully configurable (nothing is hard-coded)
  - Choose your compression level
  - Choose your hashing algorithm (BLAKE3, SHA-256)
  - Encrypt backups for multiple recipients, to decrypt backups on another
    machine (e.g. for forensic analysis)
  - Prevent untrusted backup restoration by enabling mandatory signing
- Backups are atomically restored
- Backups creation is done in a single stream, ensuring optimal execution time
- [S3 Object Lock] is supported
- [OpenPGP key passphrases] are supported
- [OpenPGP v4 and v6] are supported
- Prefix-based isolation (e.g. if backups are stored alongside other objects)
- Temporary files are automatically deleted (not relying on the OS)

Most of those features are extensively tested.

## Unsupported features

- After creation, a backup’s description cannot be changed.
  - This is inherent to how backups are stored, and the fact that most systems
    (including S3) don’t support attaching mutable free-form text.
  - We could solve this in he future (if needed) by storing a sidecar object
    containing this information, but we decided to keep things simple for now.
    - Note that we could still use the current object naming scheme, and only
      store a sidecar object if the description is changed. This would allow
      for an immutable “original description”, which is nice in terms of
      forensic analysis.
- Backups list cannot be paginated (at the storage query level).
  - Most interfaces will want to list backups in reverse chronological order,
    but backups are stored with chronological keys. Because of how S3 works,
    we cannot paginate results and instead have to query the whole list of
    backups at once.
  - We could use reversed timestamps (e.g. `9_999_999_999 - unix_timestamp`)
    in backup names so it’s already sorted in reverse chronological order.
    Given the fact that backup lists should never grow very large (otherwise
    one would have a huge object storage bill!), but we decided to keep a clear
    naming scheme.
- More encryption recipients cannot be added once the backup is signed.
  - While it would be technically possible, it would introduce a lot of
    complexity in the code and complicate the streamed upload. Adding this
    feature later would render all existing backups unrestorable (because
    integrity checks are immutable), therefore we will not implement it.
  - If you need to do this (e.g. for forensic analysis), chances are you can
    access the original encryption private key and decrypt the backup anyway.
    If we missed a use case, please [reach out].

[Prose]: https://prose.org/ "Prose IM homepage"
[Zstandard]: https://facebook.github.io/zstd/ "Zstandard homepage"
[S3 Object Lock]: https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html "“Locking objects with Object Lock” on Amazon S3 Docs"
[OpenPGP key passphrases]: https://openpgp.dev/book/private_keys.html#protecting-keys-with-passphrases "“5. Managing private key material in OpenPGP” in OpenPGP for application developers"
[OpenPGP v4 and v6]: https://openpgp.dev/book/migration.html "“17. Migration from OpenPGP v4 to v6” in OpenPGP for application developers"
[reach out]: https://prose.org/contact/ "Contact the Prose team"
