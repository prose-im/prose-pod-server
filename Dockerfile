ARG BASE_IMAGE=alpine:3.19

# -- STAGE 1 [build] --

FROM ${BASE_IMAGE} as build

WORKDIR /build

RUN apk add --no-cache \
  build-base \
  linux-headers \
  lua5.4-dev \
  libidn-dev \
  openssl-dev \
  libidn-dev

COPY ./prosody /build

COPY ./plugins/community /build/plugins
COPY ./plugins/prose /build/plugins

RUN ./configure \
  --prefix=/ \
  --sysconfdir=/etc/prosody \
  --libdir=/lib \
  --datadir=/var/lib/prosody \
  --lua-version=5.4 \
  --lua-suffix=5.4 \
  --idn-library=idn

RUN make
RUN make install

# -- STAGE 2 [run] --

FROM ${BASE_IMAGE} as run

RUN apk add --no-cache \
  libidn \
  lua5.4 \
  lua5.4-expat \
  lua5.4-socket \
  lua5.4-filesystem \
  lua5.4-sec \
  lua5.4-unbound

COPY --from=build /bin/prosody bin/
COPY --from=build /bin/prosodyctl bin/
COPY --from=build /lib/prosody/ /lib/prosody/

RUN addgroup -S prosody
RUN adduser --no-create-home -S prosody -G prosody

RUN mkdir \
  /var/lib/prosody/ \
  /var/run/prosody/

RUN chown prosody:prosody \
  /var/lib/prosody/ \
  /var/run/prosody/

VOLUME /etc/prosody/
VOLUME /var/lib/prosody/

# [public] Client connections
EXPOSE 5222/tcp

# [public] Server-to-server connections
EXPOSE 5269/tcp

# [private] HTTP
EXPOSE 5280/tcp

USER prosody:prosody

ENTRYPOINT prosody
