---
created: 2026-01-27
updated: 2026-02-02
authors:
  - Rémi Bardon <remi@remibardon.name>
---

# Prose vendor analytics

To improve Prose as a product and better match real-world needs, we need
insight about how Prose is used in practice. Since Prose can be self-hosted,
we have no way to know who is even using our products. Reaching out to end
users being therefore impossible, we have to collect a minimal amount of
information to see how Prose is used in the wild.

Our only goal is to improve Prose and assign resources on features our users
care about. We do not want to collect any personal data, and always go the
extra mile to ensure you stay in control of your data. Out of complete
transparency, this document explains in details what we collect, why we do so
and how you can configure Prose to precisely choose what you share with us.

## Our approach

Here is our approach to analytics, and the guidelines we decided to follow:

- Data SHOULD be truly anonymized (not just pseudonymized) to prevent user
  re-identification.
- Non-identifying data collection MAY be enabled by default, but configuration
  MUST exist to opt out of it.
- Identifying data collection MUST be disabled by default.
- Analytics data collection MUST be made explicit during the deployment of a
  Prose Pod to ensure configuration aligns with company requirements even
  before the first startup. This means the deployment guide MUST contain a
  warning message with guidance regarding configuration, and any tool provided
  to make deployments easier (e.g. the script hosted at `https://get.prose.org`)
  SHOULD follow the same requirements.
- Analytics data SHOULD be aggregated (i.e. removing user identifiers) when
  possible.
- Configuration MUST be made available to disable all analytics data collection
  at once, in a way that remains effective as new data points are added.
- Configuration MUST be made available to disable specific analytics data from
  being collected, in a way that remains effective as new data points are added.
- Configuration SHOULD be made available to minimize the collected data and/or
  reduce identifiability.
- Stable random values SHOULD be used in place of (even partial) hashes when
  possible, to prevent user re-identification.

### How we pseudonymize or anonymize data

Analytics events are uniqued in our analytics using one or two identifiers:
a Prose Pod identifier and an optional user identifier. The former consists of
the first 16 characters of the SHA-256 hash of the Prose Pod domain. The latter
is constructed using the same process, but with the full Jabber ID[^full-jid]
(“Full JID”) of the user. If analytics events are proxied by a Prose Pod Server
(default behavior), this user identifier is also anonymized using a random
“salt” unique to the Prose Pod (and never accessible by anyone). In this
situation, user identifiers are truly anonymous as it is technically impossible
to re-identify a user, even with additional information (e.g. already knowing
a user’s Full JID).

[^full-jid]: As defined in [RFC 6120, section 1.4](https://www.rfc-editor.org/rfc/rfc6120#section-1.4).

A pseudo-code equivalent would be:

```
pseudonymize(text):
  return first 16 characters of SHA-256(text)

anonymize(text, random_suffix):
  return pseudonymize(text + random_suffix)

pod_id(domain):
  return pseudonymize(domain)

user_id(username, domain, device_random_id, server_random_id):
  user_hash = pseudonymize(username + "@" + domain + "/" + device_random_id)
  return anonymize(user_hash, server_random_id)
```

## What we want to know

Because every data collection must start with a concrete need, this document
is organised around insights we are interested in having. Each section details
what is collected and all the configuration we’ve put at your disposal.

[`pod_version`]: #pod_version
<span id="pod_version"></span>
### What versions of Prose Pods are running in the wild

We provide a software that can be self-hosted for free without our intervention.
People can deploy it without us even knowing about it. While this fact in itself
is not a problem at all, not knowing what versions of Prose are running in the
wild forces us to assume old versions are still being used and it makes changes
more expensive to make.

Consider an example: we want to make a change that’s breaking for old versions
but not for recent ones. If we already know no one is using such an incompatible
version, or only a few people, we can just make an announcement and guide people
to migrate on an individual basis if they contact us. On the other end, if we
consider old versions are still heavily used we’ll spend hours documenting
those breaking changes which no one might ever encounter. And even then, they
might not even find/read the documentation. That’s wasted time and energy.

To answer this need, each Prose Pod Server regularly sends its own version
information to our servers. It does so at every startup then once a day.

Available configuration:

```toml
[vendor_analytics]
usage.pod_version.enabled = false # Default: true
```

[`user_app_version`]: #user_app_version
<span id="user_app_version"></span>
### What versions of Prose client apps are running in the wild

For the same reasons as we want to know what versions of Prose Pods are running
in the wild (see [`pod_version`]), we want to know the same about Prose apps
(messaging clients).

That’s why, at every app startup then once a day, Prose apps emit an analytics
event containing their version information.

Available configuration:

```toml
[vendor_analytics]
usage.user_app_version.enabled = false # Default: true
```

[`meta_user_count`]: #meta_user_count
<span id="meta_user_count"></span>
### What size are companies using Prose

Small, medium and large companies all have different needs and requirements.
Knowing the size distribution of companies using Prose would help us prioritize
certain features over others.

To have more than 20 users in their Prose Workspace (at the time of writing
this), companies already have to pay for a subscription with per-user pricing.
This means our billing system already has to know the size of all
Prose Workspaces with more than 20 users.

Below that, we have no way to know about it. That’s why we collect the number
of enabled user accounts Prose Pods have. We don’t need a lot of precision to
take decisions, so we chose sensible bins to group meaningful company sizes:
0–4, 5–9, 10–19, 20–49, 50–99, 100–199, 200–299, etc. (by hundreds above).

Prose Pod Servers send this information at startup then once a day.

Available configuration:

```toml
[vendor_analytics]
usage.meta_user_count.enabled = false # Default: true
# OR disable all analytics when below
# a certain size to avoid small cohorts.
min_cohort_size = 11
```

[`user_platform`]: #user_platform
<span id="user_platform"></span>
### What platforms users are using Prose on

Since the very [annoucement of Prose][prose-announcement], we’ve made it clear
that we want to have _native_ apps on every major platform. Doing _native_
development is a lot more time consuming, and therefore very expensive.
Before we decide to invest resources into supporting a new platform natively,
we need to know if there’s a real need for it. While people could ask for it
via our customer support channels, not a lot of people would bother doing it
and it’s easier for us to collect the platform a user is on.

This information is sent by Prose apps, at every startup then once a day.

As always, you can opt out of this using the following configuration:

```toml
[vendor_analytics]
usage.user_platform.enabled = false # Default: true
usage.user_platform.allow_list = ["web", "macos", "windows", "ios", "android"]
usage.user_platform.deny_list = ["your-secret-platform"]
```

[`user_lang`]: #user_lang
<span id="user_lang"></span>
### What languages are end-users using

At the moment, Prose softwares (client apps, administration Dashboard…) are all
mono-lingual and available in English exclusively. Of course, we plan on adding
localization in the future, but it’s not our priority at the moment
(2026-01-27). However, if we knew that a large portion of our end-users would
prefer using their primary language, we could prioritize working on that.

For example, if we notice that 20% of our end-users use German as primary
language on their device, we might consider prioritizing localization in our
roadmap. If everyone uses English as their primary language, then we can
safely assume no one will be bothered if Prose interfaces stay English-only
for a while.

For this reason, Prose apps regularly send us the preferred locales configured
on end-users’ devices. It does so at every app startup then once a day.

Available configuration:

```toml
[vendor_analytics]
usage.user_lang.enabled = false # Default: true
# How many locales to keep at most (default: max).
# Users often have mutliple locales configured. Having too many or exotic ones
# might become an identifying information.
usage.user_lang.max_locales = 1 # ≥1
```

[`pod_domain`]: #pod_domain
<span id="pod_domain"></span>
### What companies/domains are hosting Prose

It’s nice to know who uses your product. It helps understanding customer
profiles, and in some cases helps prevent illegal activities
(see [“Know Your Customer” on Wikipedia][kyc]). In our case, the least we could
want to know is the list of domains hosting our services.

Our billing system already knows this information for all Prose Workspaces with
more than 20 users, since it requires us to issue a license. However, we don’t
know anything about smaller Prose Workspaces.

Domain names being an identifying information, we made this data point “opt-in”
—meaning it needs to be explicitly enabled. If you opt into it, you can share
with us your domain name using the following configuration:

```toml
[vendor_analytics]
acquisition.pod_domain.enabled = true # Default: false
```

### How many people use Prose as an XMPP client without a Prose Pod as backend

At a high level, Prose is made of 3 parts: the client apps, the messaging
server and the administration dashboard. Although not designed for it, one can
use Prose apps as basic XMPP clients without having a Prose Pod Server serving
the messages.

To see how many people use Prose this way, Prose Pod Servers attach a litte
flag to analytics events when they are used to proxy them. By looking at the
absence of such flag, we can count who uses Prose without a Prose Pod Server.

<!--
NOTE: We cannot derive this information from domain IDs not reporting a Pod
  version (`pod_version`) as this data point can be disabled.
-->

_No configuration is available as no additional data is collected._

### If internal errors happen in the wild

No crash/bug reports are collected by Prose at the moment, but we will likely
introduce that in the future to improve the stability of Prose products.

_No configuration is available as no data is collected at the moment._

### If performance issues happen in the wild

No performance metrics are collected by Prose at the moment, but we will likely
introduce that in the future to improve the performance of Prose products.

_No configuration is available as no data is collected at the moment._

[`user_high_latency`]: #user_high_latency
<span id="user_high_latency"></span>
### How many users experience high latency

As of the date of this writing (2026-01-27), and probably for another very long
time, Prose Pods cannot be distributed. They are hosted on individual machines
and only one copy can ever exist. This means a user far from their messaging
server can suffer from high latency.

While it would be technically challenging to change make Prose Pods
horizontally scalable, we could put more time and resources into making sure
all network requests are as efficient as possible if we notice a lot of users
are experiencing high latency.

Note that we wouldn’t need to know about the end-users’ location
(see [`user_country`]) to answer that. Aggregated anonymous performance
metrics (numbers) would likely be sufficient.

_No configuration is available as no data is collected at the moment._

[abuse]: #abuse
<span id="abuse"></span>
### If people are abusing Prose

While Prose is a non-profit, people still have to be paid for the project to
move forward therefore we need a source of income. For reasons explained in
[the original annoucement of Prose][prose-announcement], we chose not to have
investors. Until we introduce a managed cloud-based offering, all of our
revenue comes from our seat-based pricing. We provide a free tier for smaller
teams, but since all of Prose is open-source there will always be people who
try their best to not pay a single dime. While we cannot technically stop them
while staying open source, we reserve the right to add some analytics to detect
abuse patterns if we ever feel the need.

## What needs we’ve considered, but decided not to address

In this section, we present needs we’ve considered but ended up not addressing.
If something didn’t make it because arguments were not convicing enough, it’s
still important for us to share it openly and keep track of the reasons why we
chose _not_ to collect any data.

[`user_country`]: #user_country
<span id="user_country"></span>
### What countries are end-users in

Knowing which countries our users are in would be a useful information to help
addressing end-users’ needs.

For example, if a lot of end-users are in Asia we could document how to deploy
Prose for their sovereign Cloud providers. We could also try to approach more
companies there if we feel like they would benefit from self-hosted
communications.

However, doing geolocation on IP addresses is quite intrusive and we figured
that users preferred locales ([`user_lang`]) and
[high latency metrics](#user_high_latency) (not yet collected) would be enough
to help us prioritize work.

<!--
Available configuration:

```toml
[vendor_analytics]
usage.user_country.enabled = false # Default: true
```

Note that if you disable `user_country`, your Prose users will appear in our
analytics as coming from your Prose Pod’s location (as your Pod Server’s IP
address would be used for geolocation).
-->

## What insights we can derive from existing data points

This list is, obviously, non-exhaustive. We will improve it on a best-effort
basis if we notice a new information we can extract from existing data points.

### How many Prose Pods are deployed

While not very useful to improve the development of Prose, we can derive the
minimum number of Prose Pods running in the wild at any given time by counting
the number of unique Prose Pod identifiers. Prose Pods where analytics have
been disabled would of course be missing, but we at least have a lower bound.

The only configuration you can set to prevent your Prose Pod from participating
in this counter is to disable analytics altogether:

```toml
[vendor_analytics]
enabled = false
```

## Data processing considerations

### IP addressed and analytics proxying

Prose apps used by your users send analytics events to your Prose Pod Server.
Your Prose Pod Server does the processing, and filters out all the data you
don’t want to share with us. Only then, it sends the data to our HTTP API
(which only sees the IP address of your Prose Pod).Mall 

## Compliance with international data protection laws (GDPR, LGPD…)

We try our best to provide configuration for as many things as possible, but we
don’t want you to have to go through all of it or be worried every time you
update Prose. For this reason, we provide configuration presets with defaults
complying with data protection laws.

For example, you can ensure your analytics confguration stays [GDPR]-compliant
by setting the following configuration:

```toml
[vendor_analytics]
preset = "gdpr"
```

The full list of available presets, and their corresponding configuration
overrides, can be found in
[`api/src/app_config.rs` of `prose-im/prose-pod-server`][presets-src].
If you need one that’s missing, please [contact our team][contact] and we’ll
try to add it as quickly as possible. Prose is open source, so if you spot an
error or would like to add a preset yourself, feel free to contribute!

More analytics metrics will be added in the future, and most (if not _all_)
will remain “opt-out”. To avoid unknowingly opting into a metric that’s enabled
by default, you MUST use a predefined preset. If you don’t, then we consider
you’re okay with “opt-out” as a default.

Although we try our best to make sure presets are and stay compliant, we are
not lawyers and might overlook some details/nuances. For this reason, we give
you the ability to override each preset to fix individual configurations if
needed. To do this, simply override a `vendor_analytics` configuration key in
`vendor_analytics.presets.<preset>`. For example:

```toml
[vendor_analytics]
preset = "gdpr"

[vendor_analytics.presets.gdpr]
usage.user_lang.max_locales = 1
```

## Compliance with company policies

Since all analytics events are proxied by Prose Pod Servers, they can be used
to enforce company policies. Prose Pod operators can configure

## Help us improve your privacy!

We try our best to find the sweet spot giving us actionable data while
protecting your privacy. If you feel like things could be improved, please
don’t come to us complaining but rather help us find a better middle ground!
Prose is a Foundation doing open source partly because we are open to changes.
Don’t hesitate [contacting the Prose team directly][contact]!

## Appendix A: Requirements Conformance

The key words “MUST”, “MUST NOT”, “REQUIRED”, “SHALL”, “SHALL NOT”, “SHOULD”,
“SHOULD NOT”, “RECOMMENDED”, “NOT RECOMMENDED”, “MAY”, and “OPTIONAL” in this
document are to be interpreted as described in [BCP 14] ([RFC 2119], [RFC 8174])
when, and only when, they appear in all capitals, as shown here.

[BCP 14]: https://tools.ietf.org/rfc/bcp/bcp14.txt
[contact]: https://prose.org/contact "Prose team contact form"
[GDPR]: https://en.wikipedia.org/wiki/General_Data_Protection_Regulation "“General Data Protection Regulation” on Wikipedia"
[kyc]: https://en.wikipedia.org/wiki/Know_your_customer "“Know your customer” on Wikipedia"
[presets-src]: https://github.com/prose-im/prose-pod-server/blob/master/api/src/app_config.rs "“prose-pod-server/api/src/app_config.rs at master · prose-im/prose-pod-server” on GitHub"
[prose-announcement]: https://prose.org/blog/introducing-prose/ "“Introducing Prose” on Prose’s blog"
[RFC 2119]: https://tools.ietf.org/rfc/rfc2119.html
[RFC 8174]: https://tools.ietf.org/rfc/rfc8174.html
