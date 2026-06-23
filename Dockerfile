# syntax=docker/dockerfile:1

# ---- Build stage ------------------------------------------------------------
FROM rust:1.86-slim-bookworm AS builder
WORKDIR /app

# Build dependencies first (better layer caching).
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && echo "" > src/lib.rs \
    && cargo build --release --locked || true
RUN rm -rf src

# Build the real binary. `rustls` is used (see Cargo.toml) so no OpenSSL needed.
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
