# Initialization phases: “initialization” vs “bootstrapping”

“Prosody initialization” refers to the process of initializing the Prosody
configuration so it has the necessary modules required for administrating via
the Prose Pod Server API (a.k.a. the “Server API”).
During this phase, only a `"localhost"` `VirtualHost` is configured, as the
Server API hasn’t loaded the Prose configuration or it is invalid (e.g. after
a factory reset) but Prosody needs at least one to function properly.

“Bootstrapping” happens after initialization, when the Server API knows the
desired XMPP server domain (via the Prose configuration). It is then able to
configure Prosody with everything the Prose Pod API (a.k.a. the “Pod API”)
needs (modules, service accounts…). Note that the bootstrapping configuration
doesn’t allow XMPP connections just yet.
When the Pod API starts, it replaces this bootstrapping configuration with one
containing all the enabled features and overrides made via the Dashboard.
Only then can users start to connect to the XMPP server.

## Why initialization and bootstrapping happen at every startup

If the Server API was able to connect to Prosody at startup, shouldn’t we just
skip the initialization phase? The answer is no, for two reasons. The first one
is that it would give admins the false idea that they can modify the Prosody
configuration by hand, although their changes will be overwritten when the
Pod API starts and when they make any change using the Prose Pod Dashboard.
The second reason is that we’d have two different code paths, which would make
the Server API less predictable. We can just keep things simple and start again
from scratch every time.
