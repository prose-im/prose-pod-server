# prose-pod-server

[![Test](https://github.com/prose-im/prose-pod-server/actions/workflows/test.yml/badge.svg?branch=master)](https://github.com/prose-im/prose-pod-server/actions/workflows/test.yml) [![Build and Release](https://github.com/prose-im/prose-pod-server/workflows/Build%20and%20Release/badge.svg)](https://github.com/prose-im/prose-pod-server/actions?query=workflow%3A%22Build+and+Release%22) [![GitHub Release](https://img.shields.io/github/v/release/prose-im/prose-pod-server.svg)](https://github.com/prose-im/prose-pod-server/releases)

**Prose Pod server source code. Depends on the official [Prosody XMPP server](https://prosody.im/) and extended for Prose requirements.**

Copyright 2023, Prose Foundation - Released under the [MIT License](./COPYING).

## Installation

The Prose Pod server is ran from its Docker image. You can find the pre-built Prose Pod server image on Docker Hub as [proseim/prose-pod-server](https://hub.docker.com/r/proseim/prose-pod-server/).

**First, pull the `proseim/prose-pod-server` image:**

```bash
docker pull proseim/prose-pod-server:latest
```

**Then, run it (feed it with its configuration and data storage directories):**

```bash
docker run --rm \
  -p 5222:5222 \
  -p 5269:5269 \
  -p 5280:5280 \
  -v /host/path/to/etc/prosody/:/etc/prosody/ \
  -v /host/path/to/var/lib/prosody/:/var/lib/prosody/ \
  proseim/prose-pod-server
```

**The following ports will be available on your host system:**

* `5222`: XMPP client-to-server port (`public` port, _open it on the public Internet_)
* `5269`: XMPP server-to-server port (`public` port, _open it on the public Internet_)
* `5280`: HTTP services, eg. WebSocket endpoint (`private` port, _keep it scoped to your host_)

👉 _The configurations can be sourced from the [prose-pod-system](https://github.com/prose-im/prose-pod-system) repository._

**If SSL certificates need to be generated, this can be done with the following commands eg.:**

```bash
openssl req \
  -x509 \
  -newkey rsa:2048 \
  -keyout /host/path/to/etc/prosody/certs/prose.org.local.key \
  -out /host/path/to/etc/prosody/certs/prose.org.local.crt \
  -sha256 \
  -days 3650 \
  -nodes \
  -subj "/CN=prose.org.local" \
  -addext "subjectAltName = DNS:groups.prose.org.local"
```

- Make sure to add the generate certificate to your keychain and mark it as trusted.
- Also, add a section in `/etc/hosts`: `127.0.0.1 prose.org.local groups.prose.org.local`

## Build

To build the Prose Pod server Docker image locally, run:

```bash
docker build -t proseim/prose-pod-server .
```

## License

Licensing information can be found in the [COPYING](./COPYING) document.

## :fire: Report A Vulnerability

If you find a vulnerability in any Prose system, you are more than welcome to report it directly to Prose Security by sending an encrypted email to [security@prose.org](mailto:security@prose.org). Do not report vulnerabilities in public GitHub issues, as they may be exploited by malicious people to target production systems running an unpatched version.

**:warning: You must encrypt your email using Prose Security GPG public key: [:key:57A5B260.pub.asc](https://files.prose.org/public/keys/gpg/57A5B260.pub.asc).**
