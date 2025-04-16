---
labels:
- Stage-Stable
summary: Redirect long messages to built-in pastebin
---

# Introduction

Pastebins are used very often in IM, especially in chat rooms. You have
a long log or command output which you need to send to someone over IM,
and don't want to fill their message window with it. Put it on a
pastebin site, and give them the URL instead, simple.

Not for everyone... no matter how hard you try, people will be unaware,
or not care. They may also be too lazy to visit a pastebin. This is
where mod_pastebin comes in!

# Details

When someone posts to a room a "large" (the actual limit is
configurable) message, Prosody will intercept the message and convert it
to a URL pointing to a built-in pastebin server. The URLs are randomly
generated, so they can be considered for most purposes to be private,
and cannot be discovered by people who are not in the room.

# Usage

To set up mod_pastebin for MUC rooms it **must** be explicitly loaded,
as in the example below - it won't work when loaded globally, as that
will only load it onto normal virtual hosts.

For example:

    Component "conference.example.com" "muc"
        modules_enabled = { "pastebin" }

Pastes will be available by default at
`http://<your-prosody>:5280/pastebin/` by default.

In Prosody 0.9 and later this can be changed with [HTTP
settings](https://prosody.im/doc/http).

In 0.8 and older this can be changed with `pastebin_ports` (see below),
or you can forward another external URL from your web server to Prosody,
use `pastebin_url` to set that URL.

# Discovery

The line and character tresholds are advertised in
[service discovery][xep-0030] like this:

``` {.xml}
<iq id="791d37e8-86d8-45df-adc2-9bcb17c45cb7" type="result" xml:lang="en" from="prosody@conference.prosody.im">
  <query xmlns="http://jabber.org/protocol/disco#info">
    <identity type="text" name="Prosŏdy IM Chatroom" category="conference"/>
    <feature var="http://jabber.org/protocol/muc"/>
    <feature var="https://modules.prosody.im/mod_pastebin"/>
    <x xmlns="jabber:x:data" type="result">
      <field type="hidden" var="FORM_TYPE">
        <value>http://jabber.org/protocol/muc#roominfo</value>
      </field>
      <field label="Title" type="text-single" var="muc#roomconfig_roomname">
        <value>Prosŏdy IM Chatroom</value>
      </field>
      <!-- etc... -->
      <field type="text-single" var="{https://modules.prosody.im/mod_pastebin}max_lines">
        <value>12</value>
      </field>
      <field type="text-single" var="{https://modules.prosody.im/mod_pastebin}max_characters">
        <value>1584</value>
      </field>
    </x>
  </query>
</iq>
```

# Configuration

  Option                    Description
  ------------------------- -------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  pastebin_threshold        Maximum length (in characters) of a message that is allowed to skip the pastebin. (default 500 characters)
  pastebin_line_threshold   The maximum number of lines a message may have before it is sent to the pastebin. (default 4 lines)
  pastebin_trigger          A string of characters (e.g. "!paste ") which if detected at the start of a message, always sends the message to the pastebin, regardless of length. (default: not set)
  pastebin_expire_after     Number of hours after which to expire (remove) a paste, defaults to 24. Set to 0 to store pastes permanently on disk.
  pastebin_ports            List of ports to run the HTTP server on, same format as mod_httpserver's http_ports[^1]
  pastebin_url              Base URL to display for pastebin links, must end with / and redirect to Prosody's built-in HTTP server[^2]

# Compatibility

  ------ -------
  trunk  Works
  0.12   Works
  0.11   Works
  0.10   Works
  0.9    Works
  0.8    Works
  ------ -------

# Todo

-   Maximum paste length
-   Web interface to submit pastes?

[^1]: As of Prosody 0.9, `pastebin_ports` is replaced by `http_ports`,
    see [Prosody HTTP server documentation](https://prosody.im/doc/http)

[^2]: See also
    [http_external_url](https://prosody.im/doc/http#external_url)
