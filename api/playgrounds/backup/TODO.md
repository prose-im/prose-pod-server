# To Do

## Known issues (to be fixed)

- Although the Pod Server restoration in itself is atomic, if the Pod API
  restoration fails we end up in an incorrect state. To prevent that we should
  let the Pod API try to restoration first and only then finish the Pod Server
  restoration.
- Fallback to SHA-256 hash if signature from an unknown key. Use cases:
  - Old backup + PGP still configured but key lost
  - PGP now configured, malicious actor plants signatures for old unsigned
    backups, rendering them unrestorable
