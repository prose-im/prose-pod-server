# prose-pod-server

[![Test](https://github.com/prose-im/prose-pod-server/actions/workflows/test.yml/badge.svg?branch=master)](https://github.com/prose-im/prose-pod-server/actions/workflows/test.yml) [![Build and Release](https://github.com/prose-im/prose-pod-server/workflows/Build%20and%20Release/badge.svg)](https://github.com/prose-im/prose-pod-server/actions?query=workflow%3A%22Build+and+Release%22)

**Prose Pod server source code. Forked from the [Prosody XMPP server](https://prosody.im/) and tailored for Prose requirements.**

Copyright 2023, Prose Foundation - Released under the [MIT License](./COPYING).

## Installation

The Prose Pod server is ran from its Docker image. You can find the pre-built Prose Pod server image on Docker Hub as [proseim/prose-pod-server](https://hub.docker.com/r/proseim/prose-pod-server/).

First, pull the `proseim/prose-pod-server` image:

```bash
docker pull proseim/prose-pod-server:latest
```

Then, run it (feed it with its configuration and data storage directories):

```bash
docker run \
  -p 5222:5222 \
  -p 5269:5269 \
  -v /path/to/your/local/etc/prosody/:/etc/prosody/ \
  -v /path/to/your/local/var/lib/prosody/:/var/lib/prosody/ \
  proseim/prose-pod-server
```

## License

Licensing information can be found in the [COPYING](./COPYING) document.

## :fire: Report A Vulnerability

If you find a vulnerability in any Prose system, you are more than welcome to report it directly to Prose Security by sending an encrypted email to [security@prose.org](mailto:security@prose.org). Do not report vulnerabilities in public GitHub issues, as they may be exploited by malicious people to target production systems running an unpatched version.

**:warning: You must encrypt your email using Prose Security GPG public key: [:key:57A5B260.pub.asc](https://files.prose.org/public/keys/gpg/57A5B260.pub.asc).**
