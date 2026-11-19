# =============================================================================
# Stage 1: Cargo Chef Planner (Cache Rust dependencies cho Render.com)
# =============================================================================
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml ./
COPY src ./src
COPY migrations ./migrations
COPY web ./web
RUN cargo chef prepare --recipe-path recipe.json

# =============================================================================
# Stage 2: Build Release Binary
# =============================================================================
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --bin bot_store_telegram

# =============================================================================
# Stage 3: Minimal Secure Runtime (Debian Bookworm Slim + Non-root user)
# =============================================================================
FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 10001 appuser

COPY --from=builder /app/target/release/bot_store_telegram /app/bot_store_telegram
COPY --from=builder /app/migrations /app/migrations
COPY --from=builder /app/web /app/web

USER appuser
EXPOSE 10000
CMD ["/app/bot_store_telegram"]
