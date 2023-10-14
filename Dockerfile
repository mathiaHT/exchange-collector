# syntax=docker/dockerfile:1.2

# Inspired by https://kerkour.com/rust-small-docker-image
ARG ALPINE_VERSION="3.15"
ARG RUST_VERSION="1.62.1"

FROM rust:${RUST_VERSION} AS builder

RUN apt-get update -y && \
    apt-get upgrade -y && \
    apt-get install -y --no-install-recommends \
        libffi-dev=3.3-6 \
        libssl-dev=1.1.1n-0+deb11u5 \
        cmake=3.18.4-2+deb11u1
RUN update-ca-certificates

# Create appuser
ENV USER=rust
ENV UID=10001

RUN adduser \
        --disabled-password \
        --gecos "" \
        --home "/nonexistent" \
        --shell "/sbin/nologin" \
        --no-create-home \
        --uid "${UID}" \
        "${USER}"

WORKDIR /home

# copy listener source code
ARG BUNDLE
ARG STACK
COPY ./artifact/${BUNDLE}/${STACK} .

RUN cargo build --release

# hadolint ignore=DL3006
FROM gcr.io/distroless/cc

# Import from builder.
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /app

# Use an unprivileged user.
USER rust:rust

# Copy our build
ARG BUNDLE
ARG STACK
COPY --chown=rust:rust ./artifact/${BUNDLE}/${STACK}/conf /app/conf
COPY --from=builder --chown=rust:rust /home/target/release/listener /app

ENTRYPOINT ["/app/listener", "run"]
