# syntax=docker/dockerfile:1.2

# Inspired by https://kerkour.com/rust-small-docker-image
ARG RUST_VERSION="1.73.0"

FROM rust:${RUST_VERSION} AS builder

RUN apt-get update -y && \
    apt-get upgrade -y && \
    apt-get install -y --no-install-recommends \
        libffi-dev=3.4.4-1 \
        libssl-dev=3.0.11-1~deb12u1 \
        cmake=3.25.1-1
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
COPY . .

RUN cargo build --release

# hadolint ignore=DL3006
FROM gcr.io/distroless/cc-debian12

# Import from builder.
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /app

# Use an unprivileged user.
USER rust:rust

# Copy our build
ARG BUNDLE
ARG STACK
COPY --chown=rust:rust ./conf /app/conf
COPY --from=builder --chown=rust:rust /home/target/release/exchange-collector /app

ENTRYPOINT ["/app/exchange-collector"]
