# syntax=docker/dockerfile:1

# ---- Build stage ------------------------------------------------------------
FROM rust:1.99-slim-trixie AS builder
WORKDIR /app

# `rustls` is used (see Cargo.toml), so no OpenSSL/system TLS libs are needed.
COPY . .
RUN cargo build --release --locked

# ---- Runtime stage ----------------------------------------------------------
FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home appuser     && mkdir /data && chown appuser /data

COPY --from=builder /app/target/release/telegram-antispam-shield \
     /usr/local/bin/telegram-antispam-shield

# Trusted members are kept in /data/antispam.db (ADR 0007). A named volume
# mounted here inherits the ownership above; without one the bot still runs
# and keeps state in a throwaway anonymous volume.
ENV ANTISPAM_DATA_DIR=/data
VOLUME /data

# Run as a non-root, least-privilege user.
USER appuser

ENTRYPOINT ["/usr/local/bin/telegram-antispam-shield"]
