---
date: 2025-11-11T19:30:00+08:00
modified: 2025-11-12T15:10:00+08:00
authors:
  - Rémi Bardon <remi@remibardon.name>
---

# Prose Pod Server architecture: Server API vs XMPP server

## Context

As detailed in [the original discussion which sparked the creation of such orchestrator][prose-pod-api#236],
we needed a way for the [Prose Pod API] (or any other HTTP API) to be able to
stop and start the [XMPP] server on request (e.g. for factory resets, backups,
migrations, etc.). However, Prose Pods are distributed as a group of isolated
[container images] and such feature would have been very complex to implement
without compromising security.

In late September 2025, as we were engineering the backups system, we decided
to run the orchestrator as part of the Prose Pod Server —avoiding the need
for complex admin authorization mechanisms at the Pod Server boundaries. This
new design would unlock a ton of features which were previously inconceivable,
while also allowing us to simplify current processes and remove overly complex
glue code.

## Architecture overview

Previously, we referred to the XMPP server as the “Prose Pod Server”, but there
is now a distinction between the two. The Prose Pod Server is now a [Rust]
program which does two things: it orchestrates the XMPP messaging server and
provides a secure HTTP API to perform admin tasks on it remotely (e.g. change
security settings, invite users, etc.). Since it is essentially a wrapper
around a XMPP server, the Prose Pod Server is conceptually divided into two
parts: its “back end” and its “front end”. The back end is the XMPP server,
while the front end is an HTTP API (the “Prose Pod Server API”, or “Server API”
for short when in the context of a Prose Pod).

The original architecture of a Prose Pod looked like this:

![Original Prose Pod (back end) architecture diagram](./assets/prose-pod-back-architecture-v1.svg)

Now, it looks like this:

![New Prose Pod (back end) architecture diagram](./assets/prose-pod-back-architecture.svg)

Although only [Prosody] can be used as a back end at the moment, we would like
to provide support for [ejabberd] too and thus try to keep a loose coupling
with the XMPP server implementation when we can. However it isn’t a prioritized
feature therefore it will likely happen in a distant future.

The Prose Pod Server itself doesn’t have a database, and relies on the XMPP
storage instead. While some future features might require us to change this,
we should try our best to keep it that way and integrate with existing XMPP
processes instead of doing our own thing on the side.

## State machine

The Prose Pod Server has its own life cycle, its own configuration and its
own state; and so does the XMPP server. To make code more predictable, more
readable and remove the need to handle all possible scenarios everywhere, we
decided to model the Server API as a [state machine].
It wasn’t trivial to setup because HTTP server frameworks are not usually
designed with this use case in mind but after some trial and error we managed
to get a clean architecture.

Without going into the details (which are explained in [`state-machine.md`]),
the most important thing is that the Server API’s state is modeled as a pair
of “back end state” and “front end state”. Meaningful pairs represent possible
states, and lifecycle actions (e.g. reload, restart, factory reset…) correcpond
to state transitions. Here is a semi-formal state diagram showing the possible
states the Server API can be in:

![Prose Pod Server API state diagram](./assets/prose-pod-server-api-state-machine.svg)

In all states except “Running” (nominal operational state), the Server API
exposes only a subset of routes, corresponding to the allowed transitions.
For example, one cannot initialize the first admin account if the XMPP server
is not running; and in that case the web server would simply not expose the
route at all and fallback to `503 Service Unavailable`.

Having such state machine makes it very safe to perform lifecycle operations
such as performing a factory reset, as the Rust compiler itself ensures we
cannot hold data from one instance to the next for example (as long as we don’t
use any static variables — which we don’t).

## High availability

Since we are aiming for relatively [high availability], we will strive to keep
the Prose Pod Server as a thin layer on top of the XMPP server, requiring as
little version bumps as possible. Restarting the binary itself would induce
downtime, which is why we provide “hot-reload” APIs instead of requiring
restarts when the static configuration file is changed. This is the reason why
the “Running with misconfiguration” state exists (see the state diagram): when
the static configuration is incorrect and the Prose Pod Server has no way to
signal the error (e.g. after receiving [`SIGHUP`]), it goes to this
inconsistent state where the XMPP server still runs as if nothing had happened
(high availability) but the Prose Pod Server API rejects requests until it is
properly reconfigured.

## Orchestrator implementation

Once again because Prose Pods are distributed as container images, the current
implementation of the Prose Pod Server runs the XMPP server as a [child process].
As part of our objective to [release Prose Pods as a single container image][single-container],
or as a next step if we’re not forced to do it during this packaging process,
we will likely add integrations with other orchestrators to allow running
Prose as a binary alongside the XMPP server (e.g. orchestrating both Prose and
Prosody via [`systemd`]).

## Configuration

The Prose Pod API uses the same configuration file as other Prose Pod
components (`/etc/prose/prose.toml`). However, it only uses a subset of it
(what it cares about). To learn more about what the Prose Pod Server uses and
supports, go read the only up-to-date source of truth: [`app_config.rs`].

[`app_config.rs`]: ../src/app_config.rs
[`SIGHUP`]: https://en.wikipedia.org/wiki/SIGHUP "SIGHUP - Wikipedia"
[`state-machine.md`]: ./state-machine.md
[`systemd`]: https://systemd.io/ "systemd Homepage"
[child process]: https://en.wikipedia.org/wiki/Child_process "Child process - Wikipedia"
[conf]: ./configuration.md "Prose Pod Server API configuration"
[container images]: https://en.wikipedia.org/wiki/Containerization_(computing) "Containerization (computing) - Wikipedia"
[ejabberd]: https://www.ejabberd.im/ "ejabberd XMPP Server Homepage"
[high availability]: https://en.wikipedia.org/wiki/High_availability "High availability - Wikipedia"
[prose-pod-api#236]: https://github.com/prose-im/prose-pod-api/discussions/236 "Orchestrate Prosody using a separate container · prose-im/prose-pod-api · Discussion #236"
[Prose Pod API]: https://github.com/prose-im/prose-pod-api "prose-im/prose-pod-api: Prose Pod API server. REST API used for administration and management."
[Prosody]: https://prosody.im/ "Prosody IM Homepage"
[Rust]: https://rust-lang.org/ "Rust Programming Language Homepage"
[single-container]: https://github.com/prose-im/prose-pod-system/issues/29 "Release as a single Docker image · Issue #29 · prose-im/prose-pod-system"
[state machine]: https://en.wikipedia.org/wiki/Finite-state_machine "Finite-state machine - Wikipedia"
[XMPP]: https://xmpp.org/ "XMPP - The universal messaging standard"
