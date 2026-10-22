# syntax=docker/dockerfile:1.7

# ---- chef: cargo-chef builds the dependency layer once, from a recipe of the manifests only ---------
# Cache mounts keep the cargo registry between builds; the cooked dependencies are a normal layer, so they
# are also reused through the registry layer cache in CI (cache mounts are not exported there).
FROM rust:1.99-slim-bookworm AS chef
ARG CHEF_VERSION=0.1.78
RUN --mount=type=cache,target=/usr/local/cargo/registry     cargo install cargo-chef --locked --version "${CHEF_VERSION}"
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
      org.opencontainers.image.description="Discordy REST API: accounts, guilds, channels, bots and webhooks" \
      org.opencontainers.image.source="https://github.com/discordy-voics/api" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.licenses="UNLICENSED"

COPY --from=build /out-api /usr/local/bin/api
ENV DISCORDY_ENV=production \
    API_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/api"]
