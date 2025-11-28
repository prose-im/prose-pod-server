module:set_global();

local measure_pending_connections = module:metric(
	"gauge", "connections_pending_outbound", "",
	"In-progress outbound s2s connections",
	{"host"}
);

local measure_pending_stanzas = module:metric(
	"gauge", "stanzas_pending_outbound", "",
	"Outbound s2s stanzas queued for delivery",
	{"host"}
);

local hosts = {};

function module.add_host(host_module)
	hosts[host_module.host] = true;
	function host_module.unload()
		hosts[host_module.host] = nil;
	end
end

module:hook("stats-update", function ()

	for host in pairs(hosts) do
		local n_pending_sessions = 0;
		local n_pending_stanzas = 0;

		for _, session in pairs(prosody.hosts[host].s2sout) do
			if session.sendq then
				n_pending_sessions = n_pending_sessions + 1;
				n_pending_stanzas = n_pending_stanzas + session.sendq:count();
			end
		end

		measure_pending_connections:with_labels(host):set(n_pending_sessions);
		measure_pending_stanzas:with_labels(host):set(n_pending_stanzas);
	end
end);
