# syntax=docker/dockerfile:1.7

# ---- chef: cargo-chef builds the dependency layer once, from a recipe of the manifests only ---------
# Cache mounts keep the cargo registry between builds; the cooked dependencies are a normal layer, so they
# are also reused through the registry layer cache in CI (cache mounts are not exported there).
# Prebuilt, checksum-pinned cargo-chef (upstream release binary) instead of compiling it from source on every
# cold build; unpacked in alpine because the slim rust image has no xz.
FROM alpine:3.22 AS chef-bin
ARG CHEF_VERSION=0.1.78
ADD --checksum=sha256:70ef940ef90d04d122f0176fdb8d6c39069191b484a1eaa29b327370c2e1c3c0 https://github.com/LukeMathWalker/cargo-chef/releases/download/v${CHEF_VERSION}/cargo-chef-x86_64-unknown-linux-gnu.tar.xz /tmp/cargo-chef.tar.xz
RUN tar -xJf /tmp/cargo-chef.tar.xz -C /tmp && mv /tmp/cargo-chef-*/cargo-chef /cargo-chef

FROM rust:1.99-slim-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends perl make && rm -rf /var/lib/apt/lists/*
COPY --from=chef-bin /cargo-chef /usr/local/cargo/bin/cargo-chef
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- build -----------------------------------------------------------------
FROM chef AS build
COPY --from=planner /src/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry     --mount=type=cache,target=/usr/local/cargo/git     cargo chef cook --locked --release -p api --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry     --mount=type=cache,target=/usr/local/cargo/git     cargo build --locked --release -p api  && cp target/release/api /out-api

# ---- runtime (distroless, non-root) ------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

ARG VERSION=0.1.0
ARG REVISION=unknown
LABEL org.opencontainers.image.title="discordy-api" \
      org.opencontainers.image.description="Voics REST API: accounts, guilds, channels, bots and webhooks" \
      org.opencontainers.image.source="https://github.com/voics-app/api" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.licenses="UNLICENSED"

COPY --from=build /out-api /usr/local/bin/api
ENV DISCORDY_ENV=production \
    API_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/api"]
