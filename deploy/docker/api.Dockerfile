# Bifrost control-plane API (#67). Multi-stage: build the release binary, then
# ship it on a slim runtime. Build context is the repo root.
FROM rust:1.82-slim-bookworm AS build
WORKDIR /src
# build-essential: SQLite is compiled from bundled C; pkg-config + ca-certificates
# for the build. (reqwest uses rustls, so no OpenSSL needed.)
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p bifrost-api

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /data bifrost \
    && mkdir -p /data && chown bifrost:bifrost /data
COPY --from=build /src/target/release/bifrost-api /usr/local/bin/bifrost-api
USER bifrost
# Listen on all interfaces inside the container (host default is 127.0.0.1).
ENV BIFROST_API_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["bifrost-api"]
