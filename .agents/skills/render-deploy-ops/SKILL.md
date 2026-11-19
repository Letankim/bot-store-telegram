---
name: render-deploy-ops
description: >-
  Use this skill when configuring Dockerfile, render.yaml Blueprint, database connection pooling, Telegram webhook registration, health checks, or deployment settings for Render.com.
---

# Render.com Deployment & Optimization Skill (`render-deploy-ops`)

Sử dụng skill này khi thiết lập hạ tầng triển khai ứng dụng Rust + PostgreSQL lên Render.com đảm bảo build nhanh, image siêu nhẹ (`< 40MB`), bảo mật và không bị mất tin nhắn.

---

## 1. Kiến trúc Chạy trên Render (Single Web Service + Render PostgreSQL)

1. **Gộp Axum API & Teloxide Bot chung 1 Port**:
   - Render Web Service chỉ expose duy nhất một cổng HTTP qua biến môi trường `PORT` (mặc định `10000`).
   - Trên môi trường Production (`BOT_MODE=webhook`), Teloxide **không mở port riêng** mà nhận update thông qua route của Axum: `POST /api/v1/webhooks/telegram`.
   - Khi khởi động (`main.rs`), nếu `BOT_MODE=webhook` và có `PUBLIC_BASE_URL`, ứng dụng tự động gọi `bot.set_webhook(url).secret_token(secret).await?`.
   - Trên môi trường Local Dev (`BOT_MODE=polling`), chạy song song `tokio::select!` giữa `axum::serve` và `teloxide::repl` / `Dispatcher::builder().dispatch()`.

2. **Tối ưu PostgreSQL Connection Pool cho Render**:
   - Gói Render Postgres cơ bản giới hạn số lượng kết nối đồng thời. Cấu hình `PgPoolOptions`:
     ```rust
     PgPoolOptions::new()
         .max_connections(config.db_max_connections) // Mặc định: 10-15
         .min_connections(2)
         .acquire_timeout(Duration::from_secs(5))
         .idle_timeout(Duration::from_secs(300))
         .max_lifetime(Duration::from_secs(1800))
         .connect(&config.database_url)
         .await?
     ```
   - Tự động chạy `sqlx::migrate!("./migrations").run(&pool).await?` khi khởi động container để đảm bảo Zero-Touch Migration trên Render.

---

## 2. Multi-stage `Dockerfile` với `cargo-chef`

Sử dụng `lukemathwalker/cargo-chef` để cache tầng dependencies giúp thời gian rebuild trên Render giảm từ ~10 phút xuống còn ~45 giây:

```dockerfile
# Stage 1: Chef Planner
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Builder
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin bot_store_telegram

# Stage 3: Minimal Runtime
FROM debian:bookworm-slim AS runtime
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
RUN useradd -m -u 10001 appuser
COPY --from=builder /app/target/release/bot_store_telegram /app/bot_store_telegram
COPY --from=builder /app/migrations /app/migrations
USER appuser
EXPOSE 10000
CMD ["/app/bot_store_telegram"]
```

---

## 3. Cấu hình `render.yaml` Chuẩn (Infrastructure as Code)

- Khai báo `healthCheckPath: /api/v1/health` để Render kiểm tra trạng thái sống của cả Web Server và kết nối PostgreSQL (`SELECT 1`).
- Toàn bộ khóa mật (`TELEGRAM_BOT_TOKEN`, `TELEGRAM_WEBHOOK_SECRET`, `INVENTORY_MASTER_KEY`, `ADMIN_JWT_SECRET`, `PAYMENT_WEBHOOK_SECRET`) đặt `sync: false` hoặc `generateValue: true` trong `render.yaml`.
