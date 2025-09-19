module:set_global()

local modulemanager = require "core.modulemanager";

local shell_env = module:shared("/*/admin_shell/env")

shell_env.groups = {};

function shell_env.groups:create(host, group_name, create_default_muc, group_id)
	local print = self.session.print;

	if not host then
		return false, "host not given"
	end

	local mod_groups = modulemanager.get_module(host, "groups_internal")
	if not mod_groups then
		return false, host .. " does not have mod_groups_internal loaded"
	end

	if not group_name then
		return false, "group name not given"
	end

	local err
	group_id, err = mod_groups.create({ name = group_name }, create_default_muc, group_id)
	if group_id then
		return true, "Created group " .. group_id
	else
		return false, err
	end
end

function shell_env.groups:sync_group(host, group_id)
	local print = self.session.print;

	if not host then
		return false, "host not given"
	end

	local mod_groups = modulemanager.get_module(host, "groups_internal")
	if not mod_groups then
		return false, host .. " does not have mod_groups_internal loaded"
	end

	if not group_id then
		return false, "group id not given"
	end

	-- Emit `group-user-added` events in case it never fired (allowing
	-- for example `mod_groups_muc_bookmarks` to inject bookmarks).
	local ok, err = mod_groups.emit_member_events(group_id)
	if not ok then
		return ok, err
	end

	-- Perform group subscriptions (e.g. if it was delayed when adding a member).
	-- NOTE: This operation is O(n^2).
	ok, err = mod_groups.sync(group_id)
	if ok then
		return true, "Synchronised members"
	else
		return ok, err
	end
end

function shell_env.groups:add_member(host, group_id, username, delay_update)
	local print = self.session.print;

	if not host then
		return false, "host not given"
	end

	local mod_groups = modulemanager.get_module(host, "groups_internal")
	if not mod_groups then
		return false, host .. " does not have mod_groups_internal loaded"
	end

	if not group_id then
		return false, "group id not given"
	end
	if not username then
		return false, "username not given"
	end

	local ok, err = mod_groups.add_member(group_id, username, delay_update)
	if ok then
		return true, "Added " .. username .. " to group " .. group_id
	else
		return ok, err
	end
end
