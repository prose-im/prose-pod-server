# To Do

## Known issues (to be fixed)

- Restoring a backup dosnloads the backup twice: one for integrity check and
  one for extraction. This can be fixed by forking (tee) the reader but make
  sure not to extract the archive before checking the signature as it might
  be a malicious archive!
- Although the Pod Server restoration in itself is atomic, if the Pod API
  restoration fails we end up in an incorrect state. To prevent that we should
  let the Pod API try to restoration first and only then finish the Pod Server
  restoration.
