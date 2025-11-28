---
labels:
- 'Stage-Beta'
summary: Restrict federation for some user roles
...

Introduction
============

This module allows you to control which user roles have permission to
communicate with remote servers.

Details
=======

There are quite a few modules that have similar functionality. The properties
of this one are:

- Uses the new [permissions and roles](https://prosody.im/doc/roles) feature introduced in Prosody 13.0
- Blocks both incoming and outgoing traffic
- Permits server-originated traffic (such as push notifications)
- Permits traffic between hosts on the same server

Configuration
=============

Simply load the module as usual:

``` {.lua}
modules_enabled = {
  ...
    "restrict_federation";
  ...
}
```

Any user without the `xmpp:federate` permission will be unable to communicate
with remote domains. By default this module does not grant this permission to
any role, meaning all users will be restricted.

To grant permission to certain roles, you can use the `add_permission`
configuration option (assuming you are using Prosody's [default authorization
module](https://prosody.im/doc/modules/mod_authz_internal)):

``` {.lua}
-- Allow registered users to federate (i.e. this excludes prosody:guest)
-- As prosody:admin inherits all permissions from this role too, admins will
-- also be able to communicate with other servers.
add_permissions = {
	["prosody:registered"] = {
		"xmpp:federate";
	};
}
```

Compatibility
=============

  ----- -------
  13.0  Works
  ----- -------
