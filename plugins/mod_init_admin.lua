-----------------------------------------------------------
-- mod_init_admin: Automatically create an admin account
-- after Prosody has started.
-- version 0.1
-----------------------------------------------------------
-- Copyright (C) 2024 Rémi Bardon <remi@remibardon.name>
--
-- This project is MIT licensed. Please see the LICENSE
-- file in the source package for more information.
-----------------------------------------------------------

local jid_prepped_split = require "prosody.util.jid".prepped_split;
local log = require "prosody.util.logger".init("init_admin");
local um = require "prosody.core.usermanager";

local prosody = _G.prosody;
local hosts = prosody.hosts;

local function init_admin()
  log("debug", "Initializing superadmin account…");

  -- Read JID from Prosody configuration
  local jid = module:get_option_string("init_admin_jid");
  if not jid then
    return false, "`init_admin_jid` must be defined in the Prosody configuration file.";
  end
  local username, host = jid_prepped_split(jid);
  if not (username and host) then
    return false, "Invalid JID. Check `init_admin_jid` in the Prosody configuration file.";
  end

  -- Check that host exists to improve error comprehension (otherwise log is just
  -- `Encountered error: /lib/prosody/core/usermanager.lua:129: attempt to index a nil value (field '?')`
  -- with a stacktrace)
  if not hosts[host] then
    return false, ("`init_admin_jid` is invalid: host `%s` doesn't exist."):format(host);
  end

  -- Read password from environment
  local var_name = module:get_option_string("init_admin_password_env_var_name", "SUPERADMIN_PASSWORD");
  local password = os.getenv(var_name);
  if not password then
    return false, ("Environment variable `%s` not defined."):format(var_name);
  end

  -- Set superadmin account role to "Server operator (full access)"
  local ok, err = um.create_user_with_role(username, password, host, "prosody:operator");
  if not ok then
    return false, ("Could not create user: %s"):format(err);
  end

  log("info", "Superadmin account created successfully");

  return true
end

-- Listen to the `"server-started"` event (defined in `util/startup.lua`),
-- sent once after the server started successfully.
-- See <https://prosody.im/doc/developers/moduleapi#modulehook_global_event_name_handler_priority>.
module:hook_global("server-started", function()
  local ok, err = init_admin();
  if not ok then
    log("error", err);
  end
end);
