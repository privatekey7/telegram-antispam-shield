# syntax=docker/dockerfile:1

# ---- Build stage ------------------------------------------------------------
FROM rust:1.88-slim-bookworm AS builder
WORKDIR /app

# `rustls` is used (see Cargo.toml), so no OpenSSL/system TLS libs are needed.
COPY . .
RUN cargo build --release --locked

# ---- Runtime stage ----------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home appuser

COPY --from=builder /app/target/release/telegram-antispam-shield \
     /usr/local/bin/telegram-antispam-shield

# Run as a non-root, least-privilege user.
USER appuser

ENTRYPOINT ["/usr/local/bin/telegram-antispam-shield"]
