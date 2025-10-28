---
labels:
- Statistics
- Stage-Alpha
summary: Cache s2s failures and prevent excessive retries
...

## Description

This module essentially rate limits the number of outgoing connection attempts
to domains that Prosody hasn't been able to connect to.

This can help with resource usage (no need to queue undelivered stanzas for
unreachable domains), and also reduces noise in the log files.

The retry period is proportional to the amount of time since Prosody last had
a connection to the domain. It also includes some randomness to prevent
multiple servers unintentially flooding a domain with reconnection attempts at
the same time. On average a failed domain will be tried around 15 times in the
first hour of unavailability, decreasing in frequency to 1-2 attempts per hour
after that.

## Configuration

The only configuration option is:

`s2s_failure_cache_size`
: The number of failed domains that will be remembered (overflows will simply
cause failures to be forgotten, and Prosody will retry those domains).

## Compatibility

Requires Prosody 13.0 or later.

For users of Prosody trunk after 13.0, commit [4067a95336dd](https://hg.prosody.im/trunk/rev/4067a95336dd)
introduces a new `s2s_block_immediate_retries` option which you may want to
enable, to catch some cases of retries which this module cannot otherwise
prevent.
