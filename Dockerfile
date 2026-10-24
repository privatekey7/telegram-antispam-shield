# syntax=docker/dockerfile:1

# ---- Build stage ------------------------------------------------------------
# Base images are pinned by digest; Dependabot (docker) bumps tag and digest.
FROM rust:1.99-slim-trixie@sha256:1a939748a7cbab172de91a014e3804a97af1394a8c844faa67fce9401c4e40fb AS builder
WORKDIR /app

# `rustls` is used (see Cargo.toml), so no OpenSSL/system TLS libs are needed.
COPY . .
RUN cargo build --release --locked

# ---- Runtime stage ----------------------------------------------------------
FROM debian:trixie-slim@sha256:62bd423aeee912639ca8ee681c2b762cb3e25137e0a92b20be455898a9f034ad AS runtime
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

# Healthy while the poller reaches Telegram: it writes a heartbeat to
# /tmp/antispam-heartbeat after every getUpdates call, and the subcommand
# checks that it is under 90 s old. No shell or curl needed.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/telegram-antispam-shield", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/telegram-antispam-shield"]
