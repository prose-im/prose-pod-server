Introduction
============

On a server with public registration it is usually desirable to prevent
registration of certain "reserved" accounts, such as "admin".

This plugin allows you to reserve individual usernames, or those
matching certain patterns. It also allows you to ensure that usernames
conform to a certain pattern.

Configuration
=============

Enable the module as any other:

    modules_enabled = {
      "block_registrations";
    }

You can then set some options to configure your desired policy:

  Option                         Default             Description
  ------------------------------ ------------------- -----------------------------------------------------------------------------------------------------------------------------------------------
  block_registrations_users      *See source code*   A list of reserved usernames
  block_registrations_matching   `{ }`               A list of [Lua patterns](http://www.lua.org/manual/5.1/manual.html#5.4.1) matching reserved usernames (slower than block_registrations_users)
  block_registrations_require    `nil`               A pattern that registered user accounts MUST match to be allowed
  block_registrations_confusable `true`              Block usernames that look similar to the reserved usernames (requires Prosody be built with ICU)

Some examples:

    block_registrations_users = { "admin", "root", "xmpp" }
    block_registrations_matching = {
      "master$" -- matches anything ending with master: postmaster, hostmaster, webmaster, etc.
    }
    block_registrations_require = "^[a-zA-Z0-9_.-]+$" -- Allow only simple ASCII characters in usernames

Compatibility
=============

  ------ -------
  13.0    Should work (please test)
  0.12    Works
  ------ -------
