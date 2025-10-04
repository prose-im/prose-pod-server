-- Prose Pod Server bootstrap configuration
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
  "auto_activate_hosts";
  "admin_shell";
  "groups_shell";
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
  }

  -- HTTP settings
  http_host = "prose-pod-server"

  -- mod_http_oauth2
  allowed_oauth2_grant_types = {
    "authorization_code";
    "refresh_token";
    "password";
  }
  oauth2_access_token_ttl = 10800
  oauth2_refresh_token_ttl = 0
  oauth2_registration_key = "{{oauth2_registration_key}}"

VirtualHost "admin.prose.local"
  admins = { "prose-pod-api@admin.prose.local" }

  -- Modules
  modules_enabled = {
    "admin_rest";
    "init_admin";
  }

  -- HTTP settings
  http_host = "prose-pod-server-admin"

  -- mod_init_admin
  init_admin_jid = "prose-pod-api@admin.prose.local"
  init_admin_password_env_var_name = "PROSE_BOOTSTRAP__PROSE_POD_API_XMPP_PASSWORD"
  init_admin_default_password = "bootstrap"
