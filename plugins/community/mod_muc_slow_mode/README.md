<!--
SPDX-FileCopyrightText: 2024 John Livingston <https://www.john-livingston.fr/>
SPDX-License-Identifier: AGPL-3.0-only
-->
# mod_muc_slow_mode

This module is a custom module that allows slow mode for MUC rooms.

This module is under AGPL-3.0 license.

There will probably be a XEP proposal for this module behaviour.

## Slow mode definition

There are some contexts in which you want to be able to rate limit MUC messages. This could have multiple motivations: avoid flooding, garantee a better readability of the room when there are hundreds of active users, …

This module propose a new option for MUC rooms, allowing room owners to fix a duration that users must wait between two messages.

There is a draft XEP for this feature, that you can find here: https://github.com/JohnXLivingston/xeps/blob/xep-slow-mode/xep-slow-mode.xml

There is a more human-readable version of this XEP here: https://livingston.frama.io/peertube-plugin-livechat/technical/slow_mode/

## Configuration

Just enable the module on your MUC component.
The feature will be accessible throught the room configuration form.

Depending on your application, it is possible that the slow mode is more important than other fields (for example for a video streaming service).
The position in the room config form can be changed be setting the option `slow_mode_duration_form_position`.
This value will be passed as priority for the "muc-config-form" hook.
By default, the field will be between muc#roomconfig_changesubject and muc#roomconfig_moderatedroom.

``` lua
VirtualHost "muc.example.com"
  modules_enabled = { "muc_slow_mode" }
  slow_mode_duration_form_position = 96
```
