-- Prose Pod Server bootstrap configuration
-- XMPP Server Configuration

---@diagnostic disable: lowercase-global, undefined-global

-- Base server configuration
pidfile = "/var/run/prosody/prosody.pid"

authentication = "internal_hashed"

default_storage = "internal"

log = {
  debug = "*console",
}

-- Network interfaces/ports
local_interfaces = { "*", "::" }
c2s_ports = { 5222 }
s2s_ports = { 5269 }
http_ports = { 5280 }
https_ports = {}

-- Modules
plugin_paths = { "/usr/local/lib/prosody/modules" }
modules_enabled = {
  "auto_activate_hosts",
  "admin_shell",
  "groups_shell",
}

-- Disable in-band registrations (done through the Prose Pod Dashboard/API)
allow_registration = false

-- Mandate highest security levels
c2s_require_encryption = true
s2s_require_encryption = true
s2s_secure_auth = false

-- Server hosts and components
VirtualHost "{{server_domain}}"
  modules_enabled = {
    "groups_internal",
  }

VirtualHost "admin.prose.local"
  admins = { "prose-pod-api@admin.prose.local" }

  -- Modules
  modules_enabled = {
    "admin_rest",
    "init_admin",
  }

  -- HTTP settings
  http_host = "prose-pod-server-admin"

  -- mod_init_admin
  init_admin_jid = "prose-pod-api@admin.prose.local"
  init_admin_password_env_var_name = "PROSE_BOOTSTRAP__PROSE_POD_API_XMPP_PASSWORD"
  init_admin_default_password = "bootstrap"
