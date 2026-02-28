# To Do

## Known issues (to be fixed)

- Although the Pod Server restoration in itself is atomic, if the Pod API
  restoration fails we end up in an incorrect state. To prevent that we should
  let the Pod API try to restoration first and only then finish the Pod Server
  restoration.
