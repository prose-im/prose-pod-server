-- Prose Pod Server initialization configuration
-- XMPP Server Configuration

---@diagnostic disable: lowercase-global, undefined-global

-- Base server configuration
pidfile = "/var/run/prosody/prosody.pid"

authentication = "internal_hashed"

default_storage = "internal"

log = {
  debug = "*console";
}

-- Network interfaces/ports
local_interfaces = { "*", "::" }
c2s_ports = { 5222 }
s2s_ports = { 5269 }
http_ports = { 5280 }
https_ports = {}

-- Modules
plugin_paths = { "/usr/local/lib/prosody/modules/" }
modules_enabled = {
  "admin_shell";
  "auto_activate_hosts";
  "groups_shell";
  -- NOTE: While we don’t use it during initialization, we need to enable
  --   `reload_modules` right away so it can reload modules when itself gets
  --   loaded. It’s an edge case, but it causes the modules loaded during
  --   initialization to not reload their configuration keys after the Pod API
  --   loads the real configuration for the first time.
  "reload_modules";
}
modules_disabled = {
  "s2s";
}

-- Disable in-band registrations (done through the Prose Pod Dashboard/API)
allow_registration = false

-- Mandate highest security levels
c2s_require_encryption = true

-- NOTE: Prosody requires at least one enabled VirtualHost to function.
VirtualHost "localhost"
