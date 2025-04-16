---
labels:
- 'Stage-Alpha'
summary: 'XEP-0474: SASL SCRAM Downgrade Protection'
...

Introduction
============

This module implements the experimental XEP-0474: SASL SCRAM Downgrade
Protection. It provides an alternative downgrade protection mechanism to
client-side pinning which is currently the most common method of downgrade
protection.

# Configuration

There are no configuration options for this module, just load it as normal.

# Compatibility

For SASL2 (XEP-0388) clients, it is compatible with the mod_sasl2 community module.

For clients using RFC 6120 SASL, it requires Prosody trunk 33e5edbd6a4a or
later. It is not compatible with Prosody 0.12 (it will load, but simply
won't do anything) for "legacy SASL".
