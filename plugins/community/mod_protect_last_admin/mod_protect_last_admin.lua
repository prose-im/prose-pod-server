module:set_global();

local um = require "prosody.core.usermanager";

local set = require "prosody.util.set";

local protected_roles = module:get_option_set("protect_last_admin_roles", { "prosody:admin", "prosody:operator" });

local errors = require "prosody.util.error".init(module.name, {
	["cannot-remove-only-admin"] = {
		code = 400;
		type = "cancel";
		condition = "service-unavailable";
		text  = "Cannot remove the only administrator";
		extra = {
			namespace = "https://prosody.im/protocol/errors";
			condition = "cannot-remove-only-admin";
		};
	};
});

local function other_protected_users(username, host)
	local protected_users = set.new();
	for protected_role in protected_roles do
		local users = um.get_users_with_role(protected_role, host);
		module:log("debug", "Role %s: %s", protected_role, require "util.serialization".serialize(users, "debug"));
		protected_users:add_list(um.get_users_with_role(protected_role, host));
	end
	protected_users:remove(username);
	return not protected_users:empty();
end

function check_last_admin(event)
	local username, host = event.username, event.host;

	local current_role = um.get_user_role(username, host);
	if not current_role then
		module:log("warn", "Couldn't detect role for %s@%s", username, host);
		return;
	end

	module:log("debug", "Checking whether to allow this event...");

	if event.role_name and protected_roles:contains(event.role_name) then
		module:log("debug", "Permitting role change to %s", event.role_name);
		return; -- Switching to another protected role is fine
	end

	if not protected_roles:contains(current_role.name) then
		module:log("debug", "Permitting event, role %s is not protected", current_role.name);
		return; -- This is not a protected user
	end

	if not other_protected_users(username, host) then
		event.reason = errors.new("cannot-remove-only-admin");
		return false;
	end
	module:log("debug", "Permitting event, no reason not to!");
end

module:hook("pre-delete-user", check_last_admin);
module:hook("pre-disable-user", check_last_admin);
module:hook("pre-user-role-change", check_last_admin);
