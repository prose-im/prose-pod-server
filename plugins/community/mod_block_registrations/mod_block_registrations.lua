local nodeprep = require "util.encodings".stringprep.nodeprep;

local normal = nodeprep;
if module:get_option_boolean("block_registrations_confusable", true) then
	local confusable = require "util.encodings".confusable;
	if not confusable then
		module:log("error", "Prosody not built with ICU, confusables mapping is unavailable.");
		module:log("error", "Rebuild or disable this feature with 'block_registrations_confusable = false'");
	end
	function normal(username)
		if username then
			username = nodeprep(username);
		end
		if username then
			username = confusable.skeleton(username);
		end
		return username;
	end
end

local block_users = module:get_option_set("block_registrations_users", {
	"abuse", "admin", "administrator", "hostmaster", "info", "news",
	"noc", "operator", "owner", "postmaster", "register", "registration",
	"root", "security", "service", "signup", "support", "sysadmin",
	"sysop", "system", "test", "trouble", "webmaster", "www", "xmpp",
}) / normal;
local block_patterns = module:get_option_set("block_registrations_matching", {});
local require_pattern = module:get_option_string("block_registrations_require");

function is_blocked(username)
	-- Check if the username is simply blocked
	if block_users:contains(username) then return true; end

	local normalized_username = normal(username);
	if block_users:contains(normalized_username) then return true; end

	for pattern in block_patterns do
		if username:find(pattern) then
			return true;
		end
	end
	-- Not blocked, but check that username matches allowed pattern
	if require_pattern and not username:match(require_pattern) then
		return true;
	end
end

module:hook("user-registering", function(event)
	local username = event.username;
	if is_blocked(username) then
		event.allowed = false;
		return true;
	end
end, 10);
