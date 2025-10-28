module:set_global();

local cache = require "prosody.util.cache";
local st = require "prosody.util.stanza";
local time = require "prosody.util.time";

local measure_failed_domains = module:metric(
	"gauge", "failed_domains", "",
	"Number of cached domain failures"
);

local measure_rejected_stanzas = module:metric(
	"counter", "failed_stanzas", "",
	"Number of rejected stanzas to failed domains"
);

local failed_domains_cache_size = module:get_option_number("s2s_failure_cache_size", 128);

local domains = cache.new(failed_domains_cache_size);

-- This function returns an appropriate delay until we should retry connecting
-- It includes randomness, so that every Prosody server does not attempt
-- to reconnect to the target server at the same time (which may overload it
-- and cause further downtime). The logic and numbers here results in approximately
-- 15 retries per hour, with up to 30min between attempts.
local function get_holdoff_time(first_disconnect, last_disconnect)
	-- Retry for a server that has been down for 2 hours will be the same
	-- range as a server that has been down for 1 hour
	local downtime = math.min(3600, last_disconnect - first_disconnect);
	-- Range from at least 10-20s up to a range proportional to the downtime
	return math.random(10 + downtime/4, 20 + downtime/2);
end

module:hook("s2sout-established", function (event)
	-- Successfully connected, so stop tracking this domain
	local domain = event.session.to_host;
	domains:set(domain, nil);
end);

module:hook("s2sout-destroyed", function (event)
	local domain = event.session.to_host;
	if not event.reason or event.session.type ~= "s2sout_unauthed" then
		return;
	end
	local current_time = time.now();
	local holdoff_time = get_holdoff_time(current_time, current_time);
	local state = domains:get(domain) or { current_time, nil, -1, current_time + holdoff_time, event.reason };
	state[2], state[3] = current_time, state[3] + 1;
	domains:set(domain, state);
	module:log("info", "Preventing further connections to %s for %d seconds", holdoff_time);
end);

module:hook("route/remote", function (event)
	local origin, stanza, domain = event.origin, event.stanza, event.to_host;
	local domain_info = domains:get(domain);
	if domain_info then
		measure_rejected_stanzas:add(1);
		local holdoff_until = domain_info[4];
		if time.now() < holdoff_until then
			return;
		end
		local reply = st.error_reply(
			stanza,
			"wait",
			"remote-server-timeout",
			domain_info[5] and ("Unreachable domain ("..domain_info[5]..")") or "Unreachable domain"
		);
		return origin.send(reply);
	end
end);

module:hook("stats-update", function ()
	measure_failed_domains:set(domains:count());
end);

module:add_item("shell-command", {
	section = "s2s";
	section_desc = "View and manage server-to-server connections";
	name = "reset_failures";
	desc = "Clear cache of connection failures";
	args = {
		{ name = "remote_host", type = "string" };
	};
	handler = function (_, remote_host)
		if remote_host then
			if domains:get(remote_host) == nil then
				return false, "No failure cached for "..remote_host;
			end
			domains:set(remote_host, nil);
			return true, "Reset failure cache for "..remote_host;
		end

		-- No remote host specified, so clear all
		domains:clear();
		return true, "Failure cache cleared";
	end;
});

module:add_item("shell-command", {
	section = "s2s";
	section_desc = "View and manage server-to-server connections";
	name = "show_failures";
	desc = "Show cache of connection failures";
	args = {
		{ name = "remote_host", type = "string" };
	};
	handler = function (shell, remote_host)
		local function show_domain_failures(domain, domain_info)
			domain_info = domain_info or domains:get(domain);
			if not domain_info then
				return false;
			end
			local first_disconnect, last_disconnect = domain_info[1], domain_info[2];
			local attempts, holdoff, reason = domain_info[3], domain_info[4], domain_info[5];

			shell.session.print(domain..":");
			shell.session.print("  First disconnected:     "..os.date("%c", first_disconnect));
			shell.session.print("  Last disconnected:      "..os.date("%c", last_disconnect));
			if reason then
				shell.session.print("            because:      "..reason);
			end
			shell.session.print("  Reconnection attempts:  "..attempts);
			shell.session.print("  No more attempts until: "..os.date("%c", last_disconnect + holdoff));

			return true;
		end
		if remote_host then
			if not show_domain_failures(remote_host) then
				return false, "No failure cached for "..remote_host;
			end
			return true, "Showing failure record for "..remote_host;
		end

		local c = 0;
		for domain, domain_info in domains:items() do
			c = c + 1;
			show_domain_failures(domain, domain_info);
		end

		return true, ("Showing %d failure records"):format(c);
	end;
});
