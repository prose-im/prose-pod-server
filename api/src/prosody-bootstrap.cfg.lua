-- Prose Pod Server bootstrap configuration
-- XMPP Server Configuration

---@diagnostic disable: lowercase-global, undefined-global

-- Base server configuration
pidfile = "/var/run/prosody/prosody.pid"
admin_socket = "/var/run/prosody/prosody.sock"

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
  -- NOTE: While we don’t use it during bootstrapping, we need to enable
  --   `reload_modules` right away so it can reload modules when itself gets
  --   loaded. It’s an edge case, but it causes the modules loaded during
  --   bootstrapping to not reload their configuration keys after the Pod API
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

-- Server hosts and components
VirtualHost "{{server_domain}}"
  modules_enabled = {
    "groups_internal";
    "http_oauth2";
    "invites";
    "rest";
    "vcard4";
  }

  -- HTTP settings
  http_host = "prose-pod-server"

  -- mod_http_oauth2
  allowed_oauth2_grant_types = {
    "authorization_code";
    "password";
    "refresh_token";
  }
  oauth2_access_token_ttl = 10800
  oauth2_refresh_token_ttl = 0
  oauth2_registration_key = ENV_OAUTH2_REGISTRATION_KEY
