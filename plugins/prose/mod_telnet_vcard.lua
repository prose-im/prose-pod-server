-----------------------------------------------------------
-- mod_telnet_vcard: Manage vcards through the telnet
-- console
-- version 0.1
-----------------------------------------------------------
-- Copyright (C) 2013 Stefan `Sec` Zehl
-- Copyright (C) 2024 Remi Bardon <remi@remibardon.name>
--
-- This project is MIT/X11 licensed. Please see the
-- COPYING file in the source package for more information.
-----------------------------------------------------------

module:set_global();
module:depends("admin_telnet");

local console_env = module:shared("/*/admin_shell/env");

console_env.vcard = {}

local storagemanager = require "prosody.core.storagemanager";
local datamanager = require "prosody.util.datamanager";
local xml = require "prosody.util.xml";
local jid = require "prosody.util.jid";
local warn = require "prosody.util.prosodyctl".show_warning;
local st = require "prosody.util.stanza"

function console_env.vcard:get(user_jid)
    local user_username, user_host = jid.split(user_jid);
    if not hosts[user_host] then
        warn("The host '%s' is not configured for this server.", user_host);
        return;
    end
    storagemanager.initialize_host(user_host);
    local vCard;
    vCard = st.deserialize(datamanager.load(user_username, user_host, "vcard"));
    if vCard then
        print(vCard);
    else
        warn("The user '%s' has no vCard configured.", user_jid);
    end
end

function console_env.vcard:set(user_jid, file)
    local user_username, user_host = jid.split(user_jid);
    if not hosts[user_host] then
        warn("The host '%s' is not configured for this server.", user_host);
        return;
    end
    storagemanager.initialize_host(user_host);
    local f = io.input(file);
    local xmldata = io.read("*all");
    io.close(f);

    local vCard = st.preserialize(xml.parse(xmldata));

    if vCard then
        datamanager.store(user_username, user_host, "vcard", vCard);
    else
        warn("Could not parse the file.");
    end
end

function console_env.vcard:delete(user_jid)
    local user_username, user_host = jid.split(user_jid);
    if not hosts[user_host] then
        warn("The host '%s' is not configured for this server.", user_host);
        return;
    end
    storagemanager.initialize_host(user_host);
    datamanager.store(user_username, user_host, "vcard", nil);
end
