# Backups and backup restoration

## General guidelines

Provide a GPG encryption key so backups are encrypted. Without it, someone with
access to your backup storage (e.g. S3) could read the contents of your backups.
They would see all your messages that are not end-to-end encrypted (e.g.
messages in Multi-User Chats).

If you don’t want backups to be encrypted, provide at least a GPG signing key.
Without it, someone with access to your backup storage (e.g. S3) could read
and tamper with your backups (e.g. add users and add or delete messages).

## General backup process

1. The Pod API receives a request to make a backup.
2. The Pod API gathers all of its data, and zips it into a tar archive.
   - Backed up: Pod API database, version info
3. The Pod API sends a backup request to the Server API, with its own data
   along the request. This way, the Server API doesn’t need to stream all
   of its data (huge, potentially insecure) back to the Pod API.
4. The Server API stops its backend (to prevent inconsistent backups).
5. The Server API

Routes:

- `POST /lifecycle/backup` -> Stop Prosody, make backup, start Prosody
  - This is safe.
- `POST /lifecycle/backup?no_downtime=true` -> Make backup without stopping Prosody
  - For now, we won’t do any flushing so this might lead to corrupted data.
- `PUT /lifecycle/restore` -> Restore backup

---

1. Stop services and flush data
2. Gather all data
3. Archive (tar)
   Use a deterministic or reproducible backup format

   Many people wrap raw data into a tar archive before compression+encryption:

   ```shell
   tar --mtime='UTC 2020-01-01' --sort=name --owner=0 --group=0 -cf backup.tar <data>
   zstd backup.tar
   gpg -o backup.tar.zst.gpg --encrypt <...>
   ```

   Benefits:

   - Single consistent file instead of multiple loose objects
   - Preserves file permissions, timestamps, symlinks
   - Can be safely extracted anywhere

   Even databases (MySQL, Postgres) often export into TAR-like structures.
4. Compress (zstd)
5. (Optional) encrypt (GPG)
6. Upload (S3)
7. Lifecycle + retention

Names: `prose_[RFC3339].tar.zst(.gpg)`
  - `prose_2025-12-02T10:46:50Z.tar.zst(.gpg)` (archived, compressed, optionally encrypted)
  - `prose_2025-12-02T10:46:50Z.tar.zst(.gpg).sig` (signature), optional
  - `prose_2025-12-02T10:46:50Z.tar.zst(.gpg).sha256` (integrity hash)

## Integrity checks

```txt
Without GPG:
  backup.tar.zst
  backup.tar.zst.sha256

With GPG signing:
  backup.tar.zst
  backup.tar.zst.sig

With GPG encryption but no signing:
  backup.tar.zst.gpg
  backup.tar.zst.gpg.sha256
    (not required if GPG encryption includes MDC/AEAD for integrity checks
    but makes processes more straightforward)

With GPG signing + encryption:
  backup.tar.zst.gpg
  backup.tar.zst.gpg.sig
```

- If backup is not signed,

## Backup encryption

### Key rotation

- Key A
- Backup 1 (key A)
- Backup 2 (key A)
- Backup 3 (key A)
- Rotation to key B
  - Keep key A
- Backup 4 (key B)

1. Normal key rotation
   1. Keep old keys and backups
2. Key leaked
   1. Delete old backups
   2. Re-encrypt old backups
3. Key lost
   1. Delete backups encrypted using this key
      -> Need a way to see this

### Attacks

No encryption
! Attacker has access to the bucket -> attacker can read data
-> Use encryption

Tampering unsigned backup
! Attacker has access to the bucket -> creates a new `.sha256` file

Tampering signed backup
! Attacker has access to the bucket -> deletes `.sig` file and creates a `.sha256` file

Tampering encrypted backup
-> Decryption fails if GPG encryption includes MDC/AEAD

Key leaked (unknown)
-> Attacker can read all backups encrypted using this key
-> Rotate frequently

Key leaked (known)
-> Delete or re-encrypt backups encrypted using this key

## Backups naming

The chosen naming convention is `prose_[RFC3339].tar.zst(.gpg)`, where
`[RFC3339]` is the [RFC 3339] timestamp with second precision and “Z” offset.
It is unlikely that collisions will ever happen, and in that case the API would
return a `409 Conflict` HTTP status code anyway.

Example: `prose_2025-12-04T16:24:32Z.tar.zst.gpg`.

Note that this file name is invalid on Windows due to the presence of colons
(`:`). This naming choice shouldn’t be an issue as backups have little reason
to be stored on a Windows machine.

---

Backups are not incremental
-> Otherwise that’s a mess
-> But gives access to all data to an attacker

We cannot store integrity checks outside the bucket as the Prose Pod Server API
has no DB and a restore would make new signatures disappear which is undesired.

[RFC 3339]: https://www.rfc-editor.org/rfc/rfc3339

---

Backups names are returned as entire file names.

This way we don’t accidentally create double-extension attack, and we don’t end
up with dupplicates while stripping extensions (e.g. `backup.tar.zst.gpg` vs
`backup.tar.zst`). Using full file names keeps unique IDs and removes ambiguity
when calling the API (imagine one wants to restore `backup.tar.zst.gpg`, but an
attacker created a malicious `backup.tar.zst` and the Server favors it because
it finds `backup.tar.zst` first!).

If we consider it hard to read for humans, we can strip extensions in GUIs and
add other indicators for “encrypted” for example. But that should happen at the
UI level.
