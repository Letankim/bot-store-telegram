# BotStoreTelegram — Master Agent Rules (`AGENTS.md`)

> **Mục tiêu dự án**: Hệ thống Bot Telegram chuyên biệt cho dịch vụ **Kích hoạt Gemini Pro chính chủ (Your Mail)** tích hợp **Telegram Mini App (WebApp Form)** thu thập thông tin chuẩn `email | password | 2FA_secret` kèm **Trang Hướng Dẫn Chi Tiết từng bước (Đóng HSTT, Kiểm tra Quốc gia Gmail, Lấy mã 2FA 32 ký tự)**, nạp tiền tự động qua cổng **PayOS (VietQR)**, ví nội bộ (`VND`) và tiếp thị liên kết (Referral).
> **Tech Stack**: Rust (`2024 edition`), `Axum` (REST API, PayOS Webhook & Telegram Mini App Server), `Teloxide` (Telegram Bot framework), `SQLx` + `PostgreSQL` (Async compile-time checked SQL), triển khai tối ưu trên **Render.com**.

---

## 1. Nguyên tắc Cốt lõi cho AI Agents (Mandatory Constraints)

1. **Zero `unwrap()` / `expect()` trong Production Code**:
   - Tuyệt đối không dùng `.unwrap()` hoặc `.expect()` trong `src/` (trừ trong `#[cfg(test)]` hoặc khi khởi tạo cấu hình lúc boot tại `main.rs`).
   - Mọi lỗi phải đi qua `Result<T, AppError>` sử dụng `thiserror` và chuyển đổi tự động sang chuẩn HTTP response thông qua `axum::response::IntoResponse`.

2. **Chuẩn hóa 100% Format API (`ApiResponse<T>`)**:
   - Mọi HTTP API endpoint dưới `/api/v1/*` bắt buộc trả về đúng 1 cấu trúc JSON duy nhất (`ApiResponse<T>`). Không bao giờ trả về chuỗi thô hay JSON tự phát. Xem chi tiết tại `.agents/rules/02-api-response-standard.md`.

3. **An toàn Tuyệt đối cho Tiền tệ & Đơn Kích Hoạt (Financial & Order Integrity)**:
   - Không bao giờ dùng `f32` / `f64` cho tiền tệ hoặc số dư ví. Bắt buộc dùng `i64` (đơn vị `VND` nguyên, ví dụ `40000` cho `40.000 đ`) hoặc `rust_decimal::Decimal` (`NUMERIC(18, 0)`).
   - Khi người dùng bấm mua/kích hoạt gói Gemini Pro:
     1. Kiểm tra số dư ví (`balance_vnd >= plan.price_vnd`) và tồn kho (`available_stock > 0`) **trước** khi cho mở Form Mini App.
     2. Khi người dùng submit Form Mini App (`email | pass | 2fa`), bắt buộc kiểm tra lại số dư + trừ tồn kho + trừ tiền ví + ghi sổ cái `wallet_transactions` + lưu đơn hàng trong **cùng một PostgreSQL Transaction (`BEGIN ... COMMIT`)**.

4. **Bảo mật Dữ liệu Nhạy cảm (`Mail|Pass|2FA`), Telegram WebApp & PayOS Webhook**:
   - Chuỗi thông tin tài khoản của khách (`customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl`) là dữ liệu cực kỳ nhạy cảm: bắt buộc mã hóa **AES-256-GCM** trước khi lưu vào bảng `activation_orders` trong PostgreSQL.
   - Mọi request từ Telegram Mini App gửi lên `/api/v1/webapp/*` bắt buộc xác thực chữ ký **`Telegram.WebApp.initData`** bằng HMAC-SHA256 với `BOT_TOKEN` (theo chuẩn Telegram WebApp Validation).
   - Mọi Webhook từ **PayOS** (`POST /api/v1/webhooks/payos`) bắt buộc xác thực chữ ký `signature` bằng **HMAC-SHA256** với `PAYOS_CHECKSUM_KEY` và đảm bảo chống cộng tiền 2 lần qua `order_code BIGINT UNIQUE`.

5. **Tối ưu hóa cho Render.com**:
   - Chạy **Single Binary** kết hợp cả `Axum HTTP Server` (API + PayOS Webhook + Static/SSR HTML cho Mini App Form & Trang Hướng Dẫn) và `Teloxide Bot` trên cùng một Tokio runtime lắng nghe trên `PORT` của Render.

---

## 2. Cấu trúc Thư mục Chuẩn (Modular Clean Architecture)

```text
BotStoreTelegram/
├── AGENTS.md                        # Quy tắc tổng quát cho mọi AI Agent
├── Cargo.toml                       # Khai báo dependencies & profile release tối ưu LTO
├── Dockerfile                       # Multi-stage build (cargo-chef + debian-slim)
├── render.yaml                      # Render Infrastructure-as-Code Blueprint
├── migrations/                      # SQLx PostgreSQL migrations
├── web/                             # Giao diện Telegram Mini App & Trang Hướng Dẫn
│   ├── activate.html                # Form nhập Email | Pass | 2FA kèm hướng dẫn trực quan
│   └── guide.html                   # Trang hướng dẫn chi tiết: Check quốc gia, Đóng HSTT, Lấy 2FA
├── .agents/
│   ├── rules/                       # Các quy tắc chuyên biệt theo từng tầng
│   └── skills/                      # Các kỹ năng (Runbooks) để Agent thực thi chuẩn xác
└── src/
    ├── main.rs                      # Entrypoint: khởi tạo Tracing, Config, DB Pool, Server
    ├── lib.rs                       # Export modules cho integration tests
    ├── config.rs                    # Quản lý biến môi trường (Telegram, PayOS, DB, AES Key)
    ├── error.rs                     # Định nghĩa AppError & ErrorCode chuẩn
    ├── response.rs                  # Định nghĩa ApiResponse<T>, PaginationMeta, ApiErrorDetail
    ├── state.rs                     # AppState (PgPool, Bot, EncryptionService, PayOsClient, Config)
    ├── crypto.rs                    # AES-256-GCM, Telegram initData HMAC, PayOS HMAC-SHA256
    ├── domain/                      # Entities, Value Objects, Validators
    │   ├── mod.rs
    │   ├── user.rs
    │   ├── wallet.rs
    │   ├── gemini_plan.rs           # Gói kích hoạt Gemini Pro (3 tháng, 6 tháng, giá, tồn kho)
    │   ├── activation_order.rs      # Đơn kích hoạt + Parser/Validator định dạng Mail|Pass|2FA
    │   └── payos.rs                 # Structs tạo link thanh toán & webhook PayOS
    ├── repo/                        # Tầng truy xuất PostgreSQL với SQLx
    │   ├── mod.rs
    │   ├── user_repo.rs
    │   ├── wallet_repo.rs
    │   ├── plan_repo.rs
    │   └── activation_repo.rs
    ├── services/                    # Business Logic & Transaction Orchestration
    │   ├── mod.rs
    │   ├── activation_service.rs    # Kiểm tra số dư, tạo đơn kích hoạt, hoàn tiền đơn lỗi
    │   ├── payos_service.rs         # Tạo link VietQR PayOS & xử lý Webhook nạp tiền tự động
    │   └── referral_service.rs      # Thưởng giới thiệu khi người mới nạp tiền lần đầu
    ├── bot/                         # Giao diện & Luồng xử lý Telegram (Teloxide)
    │   ├── mod.rs
    │   ├── commands.rs              # /start, /pixel (Kích hoạt Gemini Pro), /payadd (Nạp PayOS)
    │   ├── callbacks.rs             # Chọn gói Gemini Pro, kiểm tra ví & mở nút Mini App Form
    │   ├── keyboards.rs             # Nút WebApp mở Form Kích Hoạt, Nút Xem Hướng Dẫn, Reply Menu
    │   └── messages.rs              # Template HTML hiển thị chi tiết gói Gemini Pro giống mẫu
    └── api/                         # Axum HTTP Routers, Middlewares & Handlers
        ├── mod.rs
        ├── middleware.rs            # TraceId, Telegram WebApp Auth, Admin Auth, Rate Limit
        ├── health.rs                # GET /api/v1/health (Render healthcheck)
        ├── webapp.rs                # Phục vụ trang Form (/webapp/activate, /webapp/guide) & API submit
        ├── webhooks.rs              # POST /api/v1/webhooks/telegram & /api/v1/webhooks/payos
        └── admin/                   # Quản lý gói Gemini Pro, Xem đơn Mail|Pass|2FA đã giải mã, Hoàn tiền
```

---

## 3. Danh sách Rules & Skills Bắt buộc Tham chiếu

- **Rules**:
  - [`.agents/rules/01-architecture-rust.md`](.agents/rules/01-architecture-rust.md): Quy chuẩn viết code Rust, Axum, Teloxide và quản lý bộ nhớ.
  - [`.agents/rules/02-api-response-standard.md`](.agents/rules/02-api-response-standard.md): Quy chuẩn JSON Response cố định (`ApiResponse<T>`) và bảng mã lỗi.
  - [`.agents/rules/03-database-security-ledger.md`](.agents/rules/03-database-security-ledger.md): Quy chuẩn PostgreSQL, Ledger, AES-256-GCM cho `Mail|Pass|2FA`, xác thực Telegram WebApp `initData` và chữ ký PayOS.
- **Skills**:
  - `db-transaction-ledger`: Sử dụng khi viết giao dịch nạp tiền PayOS, kiểm tra số dư + trừ tiền khi submit form kích hoạt Gemini Pro, hoặc hoàn tiền đơn lỗi.
  - `api-endpoint-scaffold`: Sử dụng khi tạo mới hoặc chỉnh sửa bất kỳ REST API hay Webhook nào.
  - `telegram-bot-flow`: Sử dụng khi lập trình lệnh bot (`/start`, `/pixel`, `/payadd`), thẻ mô tả gói Gemini Pro, Form Mini App thu thập `Mail|Pass|2FA` và trang hướng dẫn chi tiết.
  - `payos-payment-integration`: Sử dụng khi tích hợp tạo mã thanh toán VietQR qua PayOS API và xác thực chữ ký HMAC-SHA256 từ PayOS Webhook.
  - `render-deploy-ops`: Sử dụng khi cấu hình Docker, `render.yaml`, và tối ưu tài nguyên trên Render.
