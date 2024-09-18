-- Returns affiliations of member-only rooms even for non-admins
-- Used to replace https://github.com/bjc/prosody/blob/a7799e11a9521d33cc322fa8b9cae99134219089/plugins/muc/muc.lib.lua#L1108

local jid_bare = require "prosody.util.jid".bare;
local st = require "prosody.util.stanza";
local muc = module:depends("muc");
local muc_util = module:require "muc/util";

local get_room_from_jid = muc.get_room_from_jid;
local valid_affiliations = muc_util.valid_affiliations;

module:hook("iq-get/bare/http://jabber.org/protocol/muc#admin:query", function (event)
  local origin, stanza = event.origin, event.stanza;
  local room_jid = jid_bare(stanza.attr.to);
  local room = get_room_from_jid(room_jid);

  if room and room._data.destroyed then
    return nil
  end

  if room == nil then
    return nil
  end

  local item = stanza.tags[1].tags[1];
	local _aff = item.attr.affiliation;
	local _aff_rank = valid_affiliations[_aff or "none"];
	local _rol = item.attr.role;

	if _aff and _aff_rank and not _rol then
    local reply = st.reply(stanza):query("http://jabber.org/protocol/muc#admin");
    for jid in room:each_affiliation(_aff or "none") do
      local nick = room:get_registered_nick(jid);
      reply:tag("item", {affiliation = _aff, jid = jid, nick = nick }):up();
    end
    origin.send(reply:up());
    return true;
	end
end, 1000)
