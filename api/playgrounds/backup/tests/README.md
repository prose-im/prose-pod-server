# Integration tests

## Test definitions

Test type legend:

- “✅” means the test is a happy-path.
- “🛡️” means the test ensures a guard is in place.
- “🙅” means the feature is unsupported.

### Functional tests

Those tests are at the underlying library level. They test what is technically
possible or not, not taking into account any higher-level guards. This ensures
that even if someone works around a security check (e.g. authorization) they
will be blocked anyway. Those tests are also agnostic of high-level business
logic like restoring a dowloaded backup. In that case it’ll be considered like
restoring a backup without any checksum file, and it’s the implementor’s role
to do any necessary convenience action to transform it to a supported case (e.g.
automatically computing the missing backup checksum after authorizing the user).

#### Create a backup

Test name format: `(backup_<encryption>_<signing>|restore_<case>)`, where:

- `<encryption>` cases:

  | Value | Description |
  | --- | --- |
  | `enc_none` | No encryption. |
  | `enc_gpg` | Encryption using GPG. |

| Test | Encryption | Integrity | Description |
| --- | --- | --- | --- |
| ✅ `test_roundtrip_default_config` | `none` | `hash` | Create a backup, . |
| ✅ `test_backup_enc_none_sig_gpg` | `none` | `gpg` | TODO. |
| ✅ `backup_enc_gpg_hash` | `gpg` | `hash` | TODO. |
| ✅ `backup_enc_gpg_sig_gpg` | `gpg` | `gpg` | TODO. |
| ✅ `backup_enc_none_sig_gpg_withpass` | `none` | `gpg_withpass` | Create a non encrypted backup, using passphrase-protected OpenPGP key for signing (other half of `backup_enc_gpg_withpass_sig_hash`). |
| ✅ `backup_enc_gpg_withpass_sig_hash` | `gpg_withpass` | `hash` | Create a backup, using passphrase-protected OpenPGP key for encryption and no signing (other half of `backup_enc_none_sig_gpg_withpass`). |

#### Restore a backup

Test name format: `restore_<case>`.

`<case>` can be anything, but some substrings have a particular meaning:

| Value | Description |
| --- | --- |
| `enc_none` | Backup isn’t encrypted. |
| `enc_gpg` | Backup is GPG-encrypted. |
| `_current` | Encryption or signature was made using the OpenPGP key currently used when creating backups. |
| `_expired` | Encryption or signature uses an expired OpenPGP key. |
| `_rotated` | Encryption or signature uses a valid OpenPGP key but that is no longer the one currently used when creating backups. This happens when keys are rotated. |
| `sig_none` | The backup has no external integrity check (hash or signature). Note that if the backup is GPG-encrypted, it likely contains an embedded integrity check. |
| `_withpass` | The OpenPGP key used for encryption or signature has a passphrase. |
| `_untrusted` | The OpenPGP key isn’t trusted. |

| Test | Description |
| --- | --- |
| ✅ `restore_bak_enc_none_sig_hash` | Restore a backup, not encrypted and stored alongside a hash. |
| ✅ `restore_bak_enc_none_sig_gpg_current` | TODO. |
| ✅ `restore_bak_enc_none_sig_gpg_rotated` | TODO. |
| ✅ `restore_bak_enc_none_sig_gpg_expired` | TODO. |
| ✅ `restore_bak_enc_gpg_current_sig_hash` | TODO. |
| ✅ `restore_bak_enc_gpg_rotated_sig_hash` | TODO. |
| ✅ `restore_bak_enc_gpg_expired_sig_hash` | TODO. |
| ✅ `restore_downloaded_bak` | Restore a backup that was previously downloaded. In this case we don’t have a checksum |
| ✅ `restore_bak_` | TODO |

Edge cases:

| Test | Description |
| --- | --- |
| ✅ `restore_downloaded_bak_untrusted` | TODO |
| ✅ `restore_bak_enc_unknown_sig_hash` | Restore a backup after rotating the whole encryption key and not making the old one available (rendering it unknown). |

### Threat-mitigation tests

Those tests ensure some identified attack patterns are, and will stay,
mitigated.

Note that some threat-mitigation tests are equivalent to functional tests as
they result in the same starting environment. It’s voluntary; to keep intent
clear, emphasize that some “normal” situations might actually be used
maliciously, ensure exhaustivity and similarly ensure that no functional test
is removed without realizing it was in fact preventing a certain type of attack.

#### Attack vector: Unauthorized upload to object storage

A malicious actor got upload access to the object storage (a.k.a. S3) where
backups are stored.

| Test | Description |
| --- | --- |
| 🛡️ `restore_malicious_bak_enc_none_sig_none` | TODO. |
| 🛡️ `restore_malicious_bak_enc_none_sig_hash` | Someone uploaded an unencrypted backup, with a hash. |
| 🛡️ `restore_malicious_bak_enc_none_sig_gpg_untrusted` | Someone uploaded an unencrypted backup, with a signature from an untrusted key. |
| 🛡️ `restore_malicious_bak_sig_gpg_stolen` | Someone uploaded an unencrypted backup, with a signature from a stolen key. |
| 🛡️ `restore_malicious_bak_enc_unknown_sig_hash` | Someone uploaded a backup encrypted with a stolen key, and a hash. |
| 🛡️ `restore_malicious_bak_and_hash` | TODO. |

#### Attack vector: Malicious actor got admin role

If a malicious actor gets the admin role, they can do anything. You’re screwed.

Note that this issue will subsist even after we switch to fine-grained access
control.

#### Attack vector: Malicious actor can set environment before the binary starts

This is a higher level than the library itself, but if the “Backup & Restore”
configuration can be set using environment variables (which is the case in the
Prose Pod Server) then any malicious actor that manages to set environment
variables before the configuration is loaded can trust their own key.

## Expected results

## ⚠️ Both signature and encryption disabled (default)

```toml
[backups]
# WARN: Disabling both signing and encryption is dangerous,
#   don’t do it. This is an example.
signing.enabled = false
encryption.enabled = false
```

| Scenario | `is_intact` | `is_signed` | `is_signature_valid` | `is_encrypted` | `is_encryption_valid` | `is_trusted` | `can_be_restored` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ⚠️ Both signature and encryption disabled. (default) | `true` ✅ | `false` ⚠️ | `false` ⚠️ | `false` ⚠️ | `false` ⚠️ | `true` ✅ | `true` ✅ |
| 😐 Signature is enabled, but not encryption. | `true` ✅ | `false` ⚠️ | `true` ✅ | `true` ✅ | `true` ✅ | `true` ✅ | `true` ✅ |
| . | `true` ✅ | `false` ⚠️ | `true` ✅ | `true` ✅ | `true` ✅ | `true` ✅ | `true` ✅ |
| . | `true` ✅ | `false` ⚠️ | `true` ✅ | `true` ✅ | `true` ✅ | `true` ✅ | `true` ✅ |
|  | `true` ✅ | `false` ⚠️ | `true` ✅ | `true` ✅ | `true` ✅ | `true` ✅ | `true` ✅ |

---

```toml
[backups.signing]
enabled = true
method = "gpg"
# NOTE: When signing is enabled, it becomes mandatory. Previous (non signed)
#   backups might still need to be restored, therefore one needs a way to
#   bypass the signature check. However, to prevent an attacker
trusted_checksums = ["50d858e0985ecc7f60418aaf0cc5ab587f42c2570a884095a9e8ccacd0f6545c"]

[backups.encryption]
enabled = false

[[backups.gpg.secret_keys]]
path = "/run/secrets/signing.key"
passphrase_path = "/run/secrets/signing.pass"
```

---

```toml
[backups]
# WARN: Disabling both signing and encryption is dangerous,
#   don’t do it. This is an example.
signing.enabled = false
encryption.enabled = false
```

---

Notes:

- https://www.gnupg.org/documentation/manuals/gnupg-devel/Unattended-GPG-key-generation.html
- GPG keys have subkeys. One can rotate keys by either generating a new subkey
  or an entirely new primary key. Both scenarios should be supported.
- Document the key lookup order (passphrases, expiry…).
- Signature bucket needs to be write-only (no delete).
