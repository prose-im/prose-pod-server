ARG BASE_IMAGE=alpine:3.19

# -- STAGE 1 [build] --

FROM ${BASE_IMAGE} AS build

WORKDIR /build

RUN apk add --no-cache \
  build-base \
  linux-headers \
  lua5.4-dev \
  libidn-dev \
  openssl-dev \
  libidn-dev \
  luarocks5.4 \
  sqlite-dev
RUN luarocks-5.4 install lsqlite3

COPY ./prosody /build

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

FROM ${BASE_IMAGE} AS run

RUN apk add --no-cache \
  libidn \
  lua5.4 \
  lua5.4-expat \
  lua5.4-socket \
  lua5.4-filesystem \
  lua5.4-sec \
  lua5.4-unbound \
  sqlite-libs

COPY --from=build /bin/prosody bin/
COPY --from=build /bin/prosodyctl bin/
COPY --from=build /lib/prosody/ /lib/prosody/
COPY --from=build /usr/local/lib/lua/5.4/lsqlite3.so /usr/lib/lua/5.4/

COPY ./plugins/*/ /usr/local/lib/prosody/modules/
COPY ./pod-bootstrap.cfg.lua /usr/share/prose/prosody.bootstrap.cfg.lua

RUN addgroup -S --gid 1001 prosody
RUN adduser -S --uid 1001 -G prosody --no-create-home prosody

RUN mkdir \
  /var/lib/prosody/ \
  /var/run/prosody/

RUN chown prosody:prosody \
  /var/lib/prosody/ \
  /var/run/prosody/

VOLUME /etc/prosody/
VOLUME /var/lib/prosody/
VOLUME /usr/share/prose/

# [public] Client connections
EXPOSE 5222/tcp

# [public] Server-to-server connections
EXPOSE 5269/tcp

# [private] HTTP
EXPOSE 5280/tcp

USER prosody:prosody

ENTRYPOINT ["prosody"]
