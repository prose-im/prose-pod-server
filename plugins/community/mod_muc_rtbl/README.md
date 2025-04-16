---
summary: 
rockspec:
  dependencies:
  - mod_pubsub_subscription
labels:
- Stage-Alpha
...

This module subscribes to a real-time blocklist using pubsub (XEP-0060). As
entries are added and removed from the blocklist, it immediately updates a
local service-wide ban list.

# Configuring

Load this module on your existing MUC component like so:

```lua
Component "channels.example.com" "muc"
modules_enabled = {
	-- other modules etc
	"muc_rtbl";
}
```

Then there are two options, which must be set under the component or in the
global section of your config:

```
muc_rtbl_jid = "rtbl.example"
muc_rtbl_node = "muc_bans_sha256"
```

# Compatibility

Should work with Prosody >= 0.11.x

# Developers

## Protocol

This version of mod_muc_rtbl assumes that the pubsub node contains one item
per blocked JID. The item id should be the SHA256 hash of the JID to block.
The payload is not currently used, but it is recommend to use a XEP-0377
report element as the payload.
