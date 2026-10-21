.PHONY: dev test lint build docker

IMAGE ?= ghcr.io/discordy-voics/api
VERSION ?= 0.1.0
REVISION ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)

# Runs the api against the local compose dependencies (see ../gitops/local).
dev:
	cargo run -p api

test:
	cargo test --workspace

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

build:
	cargo build --locked --release -p api

docker:
	docker build \
		--build-arg VERSION=$(VERSION) \
		--build-arg REVISION=$(REVISION) \
		-t $(IMAGE):$(VERSION) .

# Install the pre-push hook (lint + unit tests, see lefthook.yml).
.PHONY: hooks
hooks:
	lefthook install
