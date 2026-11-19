# Rule 01: Rust Architecture, Coding Standards & Performance

## 1. Nguyên tắc Phân tầng (Layered Dependency Rule)
- **Luồng phụ thuộc một chiều**: `api` / `bot` $\rightarrow$ `services` $\rightarrow$ `repo` $\rightarrow$ `domain`.
- Tầng `bot` (Teloxide) và `api` (Axum) **không được phép** viết câu lệnh SQL trực tiếp. Mọi truy vấn DB phải nằm trong `repo` và được điều phối qua `services`.
- Tầng `services` đảm nhận việc mở và đóng Transaction (`let mut tx = self.pool.begin().await?; ... tx.commit().await?;`), đảm bảo cả Telegram Bot lẫn REST API đều dùng chung một logic nghiệp vụ duy nhất.

## 2. Quy chuẩn Rust Idioms & Async/Tokio
- **Không chặn luồng Async (No Blocking in Async)**:
  - Không bao giờ gọi `std::thread::sleep`, `std::fs::*`, hoặc các phép băm/mã hóa nặng trực tiếp trên luồng worker của Tokio nếu tốn $> 1\text{ms}$. Dùng `tokio::time::sleep` hoặc `tokio::task::spawn_blocking`.
- **Quản lý State**:
  - Chia sẻ `AppState` qua `Arc<AppState>` (hoặc bọc các thành phần con có chi phí clone thấp như `PgPool`, `teloxide::Bot` vốn đã chứa `Arc` bên trong).
- **Kiểu dữ liệu chuẩn**:
  - ID thực thể: dùng `uuid::Uuid` (v4 hoặc v7) cho các bảng giao dịch/đơn hàng, và `i64` cho `telegram_id` (vì Telegram User/Chat ID là số nguyên 64-bit).
  - Thời gian: luôn dùng `chrono::DateTime<chrono::Utc>` (`TIMESTAMPTZ` trong PostgreSQL).
  - Tiền tệ: luôn dùng `rust_decimal::Decimal` (`NUMERIC(18,4)`).

## 3. Cấu hình Cargo.toml cho Production (Render Optimization)
Trong `Cargo.toml`, profile `[profile.release]` bắt buộc cấu hình tối ưu kích thước binary và tốc độ thực thi:
```toml
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "abort"
strip = true
```

## 4. Logging & Observability
- Sử dụng `tracing` và `tracing-subscriber` với định dạng JSON trên môi trường production (`RUST_LOG=info,bot_store_telegram=debug,sqlx=warn`).
- Tuyệt đối **không log** các thông tin nhạy cảm: `BOT_TOKEN`, `DATABASE_URL`, `ENCRYPTION_KEY`, mật khẩu/cookie tài khoản trong kho, hoặc token thanh toán.
- Mọi request HTTP và mọi update từ Telegram phải được gắn `request_id` / `trace_id` thông qua `#[tracing::instrument(skip(state, pool))]`.
