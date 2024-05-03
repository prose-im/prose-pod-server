-- Prosody IM
-- Copyright (C) 2008-2010 Matthew Wild
-- Copyright (C) 2008-2010 Waqas Hussain
--
-- This project is MIT/X11 licensed. Please see the
-- COPYING file in the source package for more information.
--


local config = require "prosody.core.configmanager";
local encodings = require "prosody.util.encodings";
local stringprep = encodings.stringprep;
local storagemanager = require "prosody.core.storagemanager";
local usermanager = require "prosody.core.usermanager";
local interpolation = require "prosody.util.interpolation";
local signal = require "prosody.util.signal";
local set = require "prosody.util.set";
local lfs = require "lfs";
local type = type;

local log = require "prosody.util.logger".init("prosodyctl");

local nodeprep, nameprep = stringprep.nodeprep, stringprep.nameprep;

local io, os = io, os;
local print = print;
local tonumber = tonumber;

local _G = _G;
local prosody = prosody;

local error_messages = setmetatable({
		["invalid-username"] = "The given username is invalid in a Jabber ID";
		["invalid-hostname"] = "The given hostname is invalid";
		["no-password"] = "No password was supplied";
		["no-such-user"] = "The given user does not exist on the server";
		["no-such-host"] = "The given hostname does not exist in the config";
		["unable-to-save-data"] = "Unable to store, perhaps you don't have permission?";
		["no-pidfile"] = "There is no 'pidfile' option in the configuration file, see https://prosody.im/doc/prosodyctl#pidfile for help";
		["invalid-pidfile"] = "The 'pidfile' option in the configuration file is not a string, see https://prosody.im/doc/prosodyctl#pidfile for help";
		["no-posix"] = "The mod_posix module is not enabled in the Prosody config file, see https://prosody.im/doc/prosodyctl for more info";
		["no-such-method"] = "This module has no commands";
		["not-running"] = "Prosody is not running";
		["pidfile-not-locked"] = "Could not lock Prosody's PID file";
		}, { __index = function (_,k) return "Error: "..(tostring(k):gsub("%-", " "):gsub("^.", string.upper)); end });

-- UI helpers
local show_message = require "prosody.util.human.io".printf;

local function show_usage(usage, desc)
	print("Usage: ".._G.arg[0].." "..usage);
	if desc then
		print(" "..desc);
	end
end

local function show_module_configuration_help(mod_name)
	print("Done.")
	print("If you installed a prosody plugin, don't forget to add its name under the 'modules_enabled' section inside your configuration file.")
	print("Depending on the module, there might be further configuration steps required.")
	print("")
	print("More info about: ")
	print("	modules_enabled: https://prosody.im/doc/modules_enabled")
	print("	"..mod_name..": https://modules.prosody.im/"..mod_name..".html")
end

-- Server control
local function adduser(params)
	local user, host, password = nodeprep(params.user, true), nameprep(params.host), params.password;
	if not user then
		return false, "invalid-username";
	elseif not host then
		return false, "invalid-hostname";
	end

	local host_session = prosody.hosts[host];
	if not host_session then
		return false, "no-such-host";
	end

	storagemanager.initialize_host(host);
	local provider = host_session.users;
	if not(provider) or provider.name == "null" then
		usermanager.initialize_host(host);
	end

	local ok, errmsg = usermanager.create_user(user, password, host);
	if not ok then
		return false, errmsg or "creating-user-failed";
	end
	return true;
end

local function user_exists(params)
	local user, host = nodeprep(params.user), nameprep(params.host);

	storagemanager.initialize_host(host);
	local provider = prosody.hosts[host].users;
	if not(provider) or provider.name == "null" then
		usermanager.initialize_host(host);
	end

	return usermanager.user_exists(user, host);
end

local function passwd(params)
	if not user_exists(params) then
		return false, "no-such-user";
	end

	return adduser(params);
end

local function deluser(params)
	if not user_exists(params) then
		return false, "no-such-user";
	end
	local user, host = nodeprep(params.user), nameprep(params.host);

	return usermanager.delete_user(user, host);
end

local function getpid()
	-- Get `pidfile` from the configuration
	local pidfile = config.get("*", "pidfile");
	if not pidfile then
		log("error", "Could not find `pidfile` in Prosody config.");
		return false, "no-pidfile";
	end
	if type(pidfile) ~= "string" then
		log("error", "Invalid `pidfile`: not a string.");
		return false, "invalid-pidfile";
	end

	pidfile = config.resolve_relative_path(prosody.paths.data, pidfile);

	-- Check if POSIX is supported
	local modules_disabled = set.new(config.get("*", "modules_disabled"));
	if prosody.platform ~= "posix" then
		log("error", "Cannot open the PID file: Prosody platform not POSIX.");
		return false, "no-posix";
	end
	if modules_disabled:contains("posix") then
		log("error", "Cannot open the PID file: `posix` module disabled.");
		return false, "no-posix";
	end

	-- Open file in read & write mode
	local file, err = io.open(pidfile, "r+");
	if not file then
		log("error", ("Could not open the PID file: %s"):format(err));
		return false, "pidfile-read-failed", err;
	end

	-- Lock writes to prevent race conditions
	local locked, err = lfs.lock(file, "w"); -- luacheck: ignore 411/err
	if not locked then
		file:close();
		log("error", ("Could not lock the PID file: %s"):format(err));
		return false, "pidfile-not-locked";
	end

	-- Read a PID (number) from the file
	local pid = tonumber(file:read("*a"));
	lfs.unlock(file);
	file:close();
	if not pid then
		log("error", "Invalid PID: Not a number.");
		return false, "invalid-pid";
	end

	return true, pid;
end

local function isrunning()
	local ok, pid_or_err = getpid();
	if not ok then
		local err = pid_or_err;
		if err == "pidfile-read-failed" or err == "pidfile-not-locked" then
			-- Report as not running, since we can't open the pidfile
			-- (it probably doesn't exist)
			log("error", ("Could not get Prosody's PID: %s"):format(error_messages[err]));
			return true, false;
		end
		return ok, err;
	end
	local pid = pid_or_err;

	-- Test if Prosody is running
	-- NOTE: As [kill(2)](https://man7.org/linux/man-pages/man2/kill.2.html) states:
	--   > If sig is 0, then no signal is sent, but error checking is still performed; 
	--   > this can be used to check for the existence of a process ID or process 
	--   > group ID.
	local is_running = signal.kill(pid, 0) == 0;

	return true, is_running;
end

local function start(source_dir, lua)
	lua = lua and lua .. " " or "";
	local ok, ret = isrunning();
	if not ok then
		return ok, ret;
	end
	if ret then
		return false, "already-running";
	end
	if not source_dir then
		os.execute(lua .. "./prosody -D");
	else
		os.execute(lua .. source_dir.."/../../bin/prosody -D");
	end
	return true;
end

local function stop()
	local ok, ret = isrunning();
	if not ok then
		return ok, ret;
	end
	if not ret then
		return false, "not-running";
	end

	local ok, pid = getpid()
	if not ok then return false, pid; end

	signal.kill(pid, signal.SIGTERM);
	return true;
end

local function reload()
	local ok, ret = isrunning();
	if not ok then
		return ok, ret;
	end
	if not ret then
		return false, "not-running";
	end

	local ok, pid = getpid()
	if not ok then return false, pid; end

	signal.kill(pid, signal.SIGHUP);
	return true;
end

local render_cli = interpolation.new("%b{}", function (s) return "'"..s:gsub("'","'\\''").."'" end)

local function call_luarocks(operation, mod, server)
	local dir = prosody.paths.installer;
	local ok, _, code = os.execute(render_cli("luarocks --lua-version={luav} {op} --tree={dir} {server&--server={server}} {mod?}", {
				dir = dir; op = operation; mod = mod; server = server; luav = _VERSION:match("5%.%d");
		}));
	return ok and code;
end

return {
	show_message = show_message;
	show_warning = show_message;
	show_usage = show_usage;
	show_module_configuration_help = show_module_configuration_help;
	adduser = adduser;
	user_exists = user_exists;
	passwd = passwd;
	deluser = deluser;
	getpid = getpid;
	isrunning = isrunning;
	start = start;
	stop = stop;
	reload = reload;
	call_luarocks = call_luarocks;
	error_messages = error_messages;
};
