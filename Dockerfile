ARG BASE_IMAGE=alpine:3.22.1
ARG CARGO_CHEF_IMAGE=lukemathwalker/cargo-chef:0.1.72-rust-1.89.0-alpine




FROM ${CARGO_CHEF_IMAGE} AS chef
WORKDIR /usr/src/prose-pod-server


FROM chef AS api-plan
COPY api .
RUN cargo chef prepare --recipe-path recipe.json


FROM chef AS api-build
COPY --from=api-plan /usr/src/prose-pod-server/recipe.json recipe.json

ARG CARGO_PROFILE='release'

# Build dependencies.
RUN cargo chef cook --recipe-path recipe.json --profile "${CARGO_PROFILE}"

# Build the application.
COPY api .
RUN cargo install --path . --bin prose-pod-server --profile "${CARGO_PROFILE}"




FROM ${BASE_IMAGE} AS prosody-build

WORKDIR /build

RUN apk add --no-cache \
	build-base \
	linux-headers \
	lua5.4-dev \
	libidn-dev \
	openssl-dev \
	luarocks5.4 \
	sqlite-dev

# BUG: Broken. See [Builds broken by issue in LuaSQLite3 · Issue #8 · prose-im/prose-pod-server](https://github.com/prose-im/prose-pod-server/issues/8).
#RUN luarocks-5.4 install lsqlite3

# FIX: Unzip source manually downloaded from URL defined in
# <https://luarocks.org/lsqlite3-0.9.6-1.rockspec>.
COPY ./build /build
RUN apk add --no-cache unzip
RUN unzip /build/lsqlite3_v096.zip; \
	cd /build/lsqlite3_v096; \
	luarocks-5.4 make lsqlite3-0.9.6-1.rockspec

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


FROM ${BASE_IMAGE} AS prosody-run

RUN apk add --no-cache \
	libidn \
	lua5.4 \
	lua5.4-expat \
	lua5.4-socket \
	lua5.4-filesystem \
	lua5.4-sec \
	lua5.4-unbound \
	sqlite-libs

COPY --from=prosody-build /bin/prosody bin/
COPY --from=prosody-build /bin/prosodyctl bin/
COPY --from=prosody-build /lib/prosody/ /lib/prosody/
COPY --from=prosody-build /usr/local/lib/lua/5.4/lsqlite3.so /usr/lib/lua/5.4/

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

ARG VERSION=''
ARG COMMIT=''
ARG BUILD_TIMESTAMP=''
RUN SERVER_VERSION_DIR=/lib/prosody/prose.version.d && \
	mkdir -p "${SERVER_VERSION_DIR:?}" && \
	echo "${VERSION:-}" > "${SERVER_VERSION_DIR:?}"/VERSION && \
	echo "${COMMIT:-}" > "${SERVER_VERSION_DIR:?}"/COMMIT && \
	if [ -z "${BUILD_TIMESTAMP:-}" ]; then BUILD_TIMESTAMP="$(date -u -Iseconds)" && BUILD_TIMESTAMP="${BUILD_TIMESTAMP//+00:00/Z}"; fi && \
	echo "${BUILD_TIMESTAMP:?}" > "${SERVER_VERSION_DIR:?}"/BUILD_TIMESTAMP

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




FROM prosody-run AS run

WORKDIR /usr/share/prose-pod-server

COPY --from=api-build /usr/local/cargo/bin/prose-pod-server /usr/local/bin/prose-pod-server

VOLUME /etc/prose/

ENTRYPOINT ["prose-pod-server"]

EXPOSE 8080/tcp
