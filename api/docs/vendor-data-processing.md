---
created: 2026-01-27
updated: 2026-02-02
authors:
  - Rémi Bardon <remi@remibardon.name>
---

# Prose vendor data processing

## Analytics

See [`vendor-analytics.md`](./vendor-analytics.md).

## Software updates

On platforms which support auto-updates (i.e. not web, not iOS…), Prose
client applications (e.g. Prose macOS app) can automatically perform
auto-updates. For this, they need to reach our servers to check if a new
version is available then, if applicable, download it.

As client apps are making those requests from end-users’ devices, this
reveals their IP address to our servers. While we do not collect any of this
information, it can still qualify as a personal data transfer to a third party
under certain laws. To prevent such data transfer, Prose Pod Servers act as an
anonymizing relay for those requests.

If Prose client applications are used without a Prose Pod as back end, then
requests are still made directly to our servers. To prevent this, auto-updates
must be disabled in the app’s settings.

Note that auto-updates can be disabled altogether in a Prose Pod’s
configuration to prevent checks and downloads from happening at all.
Read [“Pod configuration reference” in the Prose Technical Docs][config-ref]
for more information.

[config-ref]: https://docs.prose.org/references/pod-config/
