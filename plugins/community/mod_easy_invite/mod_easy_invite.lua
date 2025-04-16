-- XEP-0401: Easy User Onboarding
local dataforms = require "util.dataforms";
local datetime = require "util.datetime";
local jid_bare = require "util.jid".bare;
local jid_split = require "util.jid".split;
local split_jid = require "util.jid".split;
local rostermanager = require "core.rostermanager";
local modulemanager = require "core.modulemanager";
local st = require "util.stanza";

local invite_only = module:get_option_boolean("registration_invite_only", true);
local require_encryption = module:get_option_boolean("c2s_require_encryption",
	module:get_option_boolean("require_encryption", false));

local new_adhoc = module:require("adhoc").new;

-- Whether local users can invite other users to create an account on this server
local allow_user_invites = module:get_option_boolean("allow_user_invites", true);

local invites;
if prosody.shutdown then -- COMPAT hack to detect prosodyctl
	invites = module:depends("invites");
end

local invite_result_form = dataforms.new({
		title = "Your Invite",
		-- TODO instructions = something helpful
		{
			name = "uri";
			label = "Invite URI";
			-- TODO desc = something helpful
		},
		{
			name = "url" ;
			var = "landing-url";
			label = "Invite landing URL";
		},
		{
			name = "expire";
			label = "Token valid until";
		},
	});

module:depends("adhoc");
module:provides("adhoc", new_adhoc("New Invite", "urn:xmpp:invite#invite",
		function (_, data)
			local username = split_jid(data.from);
			local invite = invites.create_contact(username, allow_user_invites);
			--TODO: check errors
			return {
				status = "completed";
				form = {
					layout = invite_result_form;
					values = {
						uri = invite.uri;
						url = invite.landing_page;
						expire = datetime.datetime(invite.expires);
					};
				};
			};
		end, "local_user"));


-- TODO
-- module:provides("adhoc", new_adhoc("Create account", "urn:xmpp:invite#create-account", function () end, "admin"));

-- XEP-0379: Pre-Authenticated Roster Subscription
module:hook("presence/bare", function (event)
	local stanza = event.stanza;
	if stanza.attr.type ~= "subscribe" then return end

	local preauth = stanza:get_child("preauth", "urn:xmpp:pars:0");
	if not preauth then return end
	local token = preauth.attr.token;
	if not token then return end

	local username, host = jid_split(stanza.attr.to);

	local invite, err = invites.get(token, username);

	if not invite then
		module:log("debug", "Got invalid token, error: %s", err);
		return;
	end

	local contact = jid_bare(stanza.attr.from);

	module:log("debug", "Approving inbound subscription to %s from %s", username, contact);
	if rostermanager.set_contact_pending_in(username, host, contact, stanza) then
		if rostermanager.subscribed(username, host, contact) then
			invite:use();
			rostermanager.roster_push(username, host, contact);

			-- Send back a subscription request (goal is mutual subscription)
			if not rostermanager.is_user_subscribed(username, host, contact)
			and not rostermanager.is_contact_pending_out(username, host, contact) then
				module:log("debug", "Sending automatic subscription request to %s from %s", contact, username);
				if rostermanager.set_contact_pending_out(username, host, contact) then
					rostermanager.roster_push(username, host, contact);
					module:send(st.presence({type = "subscribe", to = contact }));
				else
					module:log("warn", "Failed to set contact pending out for %s", username);
				end
			end
		end
	end
end, 1);

-- TODO sender side, magic automatic mutual subscription

local invite_stream_feature = st.stanza("register", { xmlns = "urn:xmpp:invite" }):up();
module:hook("stream-features", function(event)
	local session, features = event.origin, event.features;

	-- Advertise to unauthorized clients only.
	if session.type ~= "c2s_unauthed" or (require_encryption and not session.secure) then
		return
	end

	features:add_child(invite_stream_feature);
end);

-- Client is submitting a preauth token to allow registration
module:hook("stanza/iq/urn:xmpp:pars:0:preauth", function(event)
	local preauth = event.stanza.tags[1];
	local token = preauth.attr.token;
	local validated_invite = invites.get(token);
	if not validated_invite then
		local reply = st.error_reply(event.stanza, "cancel", "forbidden", "The invite token is invalid or expired");
		event.origin.send(reply);
		return true;
	end
	event.origin.validated_invite = validated_invite;
	local reply = st.reply(event.stanza);
	event.origin.send(reply);
	return true;
end);

-- Registration attempt - ensure a valid preauth token has been supplied
module:hook("user-registering", function (event)
	local validated_invite = event.validated_invite or (event.session and event.session.validated_invite);
	if invite_only and not validated_invite then
		event.allowed = false;
		event.reason = "Registration on this server is through invitation only";
		return;
	end
	if validated_invite.additional_data and validated_invite.additional_data.allow_reset then
		event.allow_reset = validated_invite.additional_data.allow_reset;
	end
end);

-- Make a *one-way* subscription. User will see when contact is online,
-- contact will not see when user is online.
function subscribe(host, user_username, contact_username)
	local user_jid = user_username.."@"..host;
	local contact_jid = contact_username.."@"..host;
	-- Update user's roster to say subscription request is pending...
	rostermanager.set_contact_pending_out(user_username, host, contact_jid);
	-- Update contact's roster to say subscription request is pending...
	rostermanager.set_contact_pending_in(contact_username, host, user_jid);
	-- Update contact's roster to say subscription request approved...
	rostermanager.subscribed(contact_username, host, user_jid);
	-- Update user's roster to say subscription request approved...
	rostermanager.process_inbound_subscription_approval(user_username, host, contact_jid);
end

-- Make a mutual subscription between jid1 and jid2. Each JID will see
-- when the other one is online.
function subscribe_both(host, user1, user2)
	subscribe(host, user1, user2);
	subscribe(host, user2, user1);
end

-- Registration successful, if there was a preauth token, mark it as used
module:hook("user-registered", function (event)
	local validated_invite = event.validated_invite or (event.session and event.session.validated_invite);
	if not validated_invite then
		return;
	end
	local inviter_username = validated_invite.inviter;
	local contact_username = event.username;
	validated_invite:use();

	if inviter_username then
		module:log("debug", "Creating mutual subscription between %s and %s", inviter_username, contact_username);
		subscribe_both(module.host, inviter_username, contact_username);
	end

	if validated_invite.additional_data then
		module:log("debug", "Importing roles from invite");
		local roles = validated_invite.additional_data.roles;
		if roles then
			module:open_store("roles"):set(contact_username, roles);
		end
	end
end);

-- Equivalent of user-registered but for when the account already existed
-- (i.e. password reset)
module:hook("user-password-reset", function (event)
	local validated_invite = event.validated_invite or (event.session and event.session.validated_invite);
	if not validated_invite then
		return;
	end
	validated_invite:use();
end);

do
	-- Telnet command
	-- Since the telnet console is global this overwrites the command for
	-- each host it's loaded on, but this should be fine.

	local get_module = require "core.modulemanager".get_module;

	local console_env = module:shared("/*/admin_telnet/env");

	-- luacheck: ignore 212/self
	console_env.invite = {};
	function console_env.invite:create_account(user_jid)
		local username, host = jid_split(user_jid);
		local mod_invites, err = get_module(host, "invites");
		if not mod_invites then return nil, err or "mod_invites not loaded on this host"; end
		local invite, err = mod_invites.create_account(username);
		if not invite then return nil, err; end
		return true, invite.uri;
	end

	function console_env.invite:create_contact(user_jid, allow_registration)
		local username, host = jid_split(user_jid);
		local mod_invites, err = get_module(host, "invites");
		if not mod_invites then return nil, err or "mod_invites not loaded on this host"; end
		local invite, err = mod_invites.create_contact(username, allow_registration);
		if not invite then return nil, err; end
		return true, invite.uri;
	end
end

local sm = require "core.storagemanager";
function module.command(arg)
	if #arg < 2 or arg[2] ~= "generate" then
		print("usage: prosodyctl mod_easy_invite example.net generate");
		return;
	end

	local host = arg[1];
	assert(hosts[host], "Host "..tostring(host).." does not exist");
	sm.initialize_host(host);

	-- Load mod_invites
	invites = module:context(host):depends("invites");
	local invites_page_module = module:context(host):get_option_string("easy_invite_page_module", "invites_page");
	if modulemanager.get_modules_for_host(host):contains(invites_page_module) then
		module:context(host):depends(invites_page_module);
	end

	table.remove(arg, 1);
	table.remove(arg, 1);

	local invite, roles;
	if arg[1] == "--reset" then
		local nodeprep = require "util.encodings".stringprep.nodeprep;
		local username = nodeprep(arg[2]);
		if not username then
			print("Please supply a valid username to generate a reset link for");
			return;
		end
		invite = invites.create_account_reset(username);
	else
		if arg[1] == "--admin" then
			roles = { ["prosody:admin"] = true };
		elseif arg[1] == "--role" then
			roles = { [arg[2]] = true };
		end
		invite = invites.create_account(nil, { roles = roles });
	end

	print(invite.landing_page or invite.uri);
end
