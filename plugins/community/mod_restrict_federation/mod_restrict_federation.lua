local jid_node = require "prosody.util.jid".node;
local st = require "prosody.util.stanza";

local get_user_role = require "prosody.core.usermanager".get_user_role;

function check_outgoing_stanza(event)
	local origin, stanza = event.origin, event.stanza;

	if not origin or origin.type ~= "c2s" then
		-- We only filter user-originated traffic, so
		-- pass this through.
		return;
	end

	if module:may("xmpp:federate", event) then
		-- Pass through
		return;
	end

	-- Block
	module:log("debug", "Forbidding outgoing %s stanza from <%s> to <%s>", stanza.name, stanza.attr.from, stanza.attr.to);
	local err_reply = st.error_reply(event.stanza, "auth", "policy-violation", "Communication with remote domains is not permitted");
	origin.send(err_reply);
	return true;
end

function check_incoming_stanza(event)
	local origin, stanza = event.origin, event.stanza;

	if origin.type ~= "s2sin" then
		-- We only filter incoming traffic from remote domains
		-- Pass through
		return;
	end

	local recipient_username = jid_node(stanza.attr.to);
	if not recipient_username then
		return;
	end

	local recipient_role, role_err = get_user_role(recipient_username, module.host);
	if not recipient_role then
		module:log("warn", "Unable to determine recipient role: %s", role_err);
		-- No idea what the role is, we'll pass it through
		return;
	end

	if recipient_role:may("xmpp:federate") then
		-- Allowed, pass through
		return;
	end

	-- Block
	module:log("debug", "Forbidding incoming %s stanza from <%s> to <%s>", stanza.name, stanza.attr.from, stanza.attr.to);
	local err_reply = st.error_reply(event.stanza, "cancel", "service-unavailable");
	origin.send(err_reply);
	return true;
end

module:hook("message/bare", check_incoming_stanza, 500);
module:hook("message/full", check_incoming_stanza, 500);
module:hook("presence/bare", check_incoming_stanza, 500);
module:hook("presence/full", check_incoming_stanza, 500);
module:hook("iq/bare", check_incoming_stanza, 500);
module:hook("iq/full", check_incoming_stanza, 500);

module:hook("route/remote", check_outgoing_stanza, 500);
