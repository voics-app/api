# syntax=docker/dockerfile:1

# ---- build -----------------------------------------------------------------
FROM rust:1.99-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p api \
 && cp target/release/api /out-api

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
