---
labels:
- 'Stage-Beta'
summary: Prevent the last admin account from being deleted or demoted
...

Introduction
============

This module ensures there is always one administrator (or one account with the
specified roles, which can be configured). It prevents a situation where the
only admin on a server deletes their account or switches to another role,
leaving the server in a state where it has no administrator and no
administrator can be created with manual intervention (e.g. using prosodyctl
to set someone as an admin).

Configuration
=============

Simply load the module as usual:

``` {.lua}
modules_enabled = {
  ...
    "protect_last_admin";
  ...
}
```

By default the module ensures there is at least one account with the
`prosody:operator` or `prosody:admin` role. You can specify the list of roles
using the `protect_last_admin_roles` setting in the configuration:

``` {.lua}
-- Ensure there is always one prosody:operator
protect_last_admin_roles = { "prosody:operator" }
```

Compatibility
=============

  ----- -------
  trunk  Works (requires commit [01f95f3de6fc](https://hg.prosody.im/trunk/rev/01f95f3de6fc) from 2025-09-25)
  ----- -------
  13.0   Not compatible
  ----  -------
