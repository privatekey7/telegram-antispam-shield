# syntax=docker/dockerfile:1

# ---- Build stage ------------------------------------------------------------
# Base images are pinned by digest; Dependabot (docker) bumps tag and digest.
FROM rust:1.99-slim-trixie@sha256:1a939748a7cbab172de91a014e3804a97af1394a8c844faa67fce9401c4e40fb AS builder
WORKDIR /app

# `rustls` is used (see Cargo.toml), so no OpenSSL/system TLS libs are needed.
COPY . .
RUN cargo build --release --locked \
    && mkdir /app/data-dir

# ---- Runtime stage ----------------------------------------------------------
# Distroless: glibc, CA certificates and nothing else. No shell, no package
# manager, so a compromised process has no tools to work with. Debian 13
# matches the builder's glibc.
FROM gcr.io/distroless/cc-debian13@sha256:d74d144baa124dd9defd01d71872bb56c49d4441dba6153ccf790364f97e64b1 AS runtime

COPY --from=builder /app/target/release/telegram-antispam-shield \
     /usr/local/bin/telegram-antispam-shield

# Trusted members are kept in /data/antispam.db (ADR 0007). A named volume
# mounted here inherits this ownership; without one the bot still runs and
# keeps state in a throwaway anonymous volume. UID 10001, as in earlier
# images, so existing volumes stay writable.
COPY --from=builder --chown=10001:10001 /app/data-dir /data
ENV ANTISPAM_DATA_DIR=/data
VOLUME /data

# Run as a non-root, least-privilege user (numeric: distroless has no
# useradd, and none is needed).
USER 10001:10001

# Healthy while the poller reaches Telegram: it writes a heartbeat to
# /tmp/antispam-heartbeat after every getUpdates call, and the subcommand
# checks that it is under 90 s old. No shell or curl needed.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/telegram-antispam-shield", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/telegram-antispam-shield"]
