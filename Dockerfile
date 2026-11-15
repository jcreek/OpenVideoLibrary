FROM --platform=$BUILDPLATFORM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
COPY scripts/copy-contract.mjs /src/scripts/copy-contract.mjs
COPY docs/openapi.json /src/docs/openapi.json
RUN npm run build

# Build natively for TARGETPLATFORM (Buildx/QEMU for a different host arch).
FROM rust:1.99.1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release --package ovl-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg ca-certificates vainfo mesa-va-drivers && if [ "$(dpkg --print-architecture)" = amd64 ]; then apt-get install -y --no-install-recommends intel-media-va-driver; fi && rm -rf /var/lib/apt/lists/* && useradd -u 10001 -m ovl && mkdir /data && chown ovl /data
COPY --from=server /src/target/release/ovl-server /usr/local/bin/ovl
COPY --from=web /src/web/dist /opt/ovl/web
COPY LICENSE /usr/share/doc/openvideolibrary/LICENSE
LABEL org.opencontainers.image.title="OpenVideoLibrary"
LABEL org.opencontainers.image.licenses="AGPL-3.0-only"
USER ovl
ENV OVL_DATA=/data OVL_LISTEN=:8096 OVL_WEB=/opt/ovl/web XDG_CACHE_HOME=/data/cache
VOLUME ["/data"]
EXPOSE 8096
ENTRYPOINT ["ovl"]
