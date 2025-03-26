-----------------------------------------------------------
-- mod_init_admin: Automatically create an admin account
-- after Prosody has started.
-- version 0.1
-----------------------------------------------------------
-- Copyright (C) 2024–2025 Rémi Bardon <remi@remibardon.name>
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

  local role = "prosody:operator";

  if host == module.host then
    -- Check that host exists to improve error comprehension (otherwise log is just
    -- `Encountered error: /lib/prosody/core/usermanager.lua:129: attempt to index a nil value (field '?')`
    -- with a stacktrace)
    if not hosts[host] then
      return false, ("`init_admin_jid` is invalid: host `%s` doesn't exist."):format(host);
    end

    -- Read password from environment
    local password = module:get_option_string("init_admin_password");
    if not password then
      local var_name = module:get_option_string("init_admin_password_env_var_name", "SUPERADMIN_PASSWORD");
      password = os.getenv(var_name);
      if not password then
        return false, ("Environment variable `%s` not defined."):format(var_name);
      end
    end

    -- Set superadmin account role to "Server operator (full access)"
    local ok, err = um.create_user_with_role(username, password, host, role);
    if not ok then
      return false, ("Could not create user: %s"):format(err);
    end

    log("info", "Superadmin account created successfully");
  else
    local ok, err = um.set_jid_role(jid, module.host, role);
    if not ok then
      return false, ("Could not grant role '%s' for host '%s' to '%s': %s"):format(role, module.host, jid, err);
    end

    log("info", ("Superadmin account '%s' was successfully granted the role '%s' for host '%s'"):format(jid, role, host));
  end

  return true
end

-- `module.ready` runs when the module is loaded and the server has finished starting up.
-- See `core/modulemanager.lua`.
function module.ready()
  local ok, err = init_admin();
  if not ok then
    log("error", err);
  end
end
