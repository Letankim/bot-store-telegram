# BotStoreTelegram — Bot Kích Hoạt Gemini Pro Chính Chủ (Rust + PostgreSQL + PayOS + Render)

Hệ thống Telegram Bot + Mini App Form (`email | pass | 2FA`) + Trang Hướng Dẫn 4 bước + Nạp tiền tự động PayOS (VietQR), viết bằng **Rust (`Axum` + `Teloxide` + `SQLx`)**, tối ưu triển khai Single Binary trên **Render.com**.

---

## 1. Kiến trúc & Tính năng Chính

- **Kích hoạt Gemini Pro Chính Chủ (Your Mail)**:
  - Hiển thị thẻ gói sản phẩm (`Giá 40.000 đ`, `Tồn kho`, `Đã bán`, khung mô tả chi tiết, link hướng dẫn).
  - Kiểm tra số dư ví trước khi cho mở Form Mini App. Nếu thiếu tiền $\rightarrow$ tự động gợi ý nạp đúng số tiền thiếu qua PayOS.
- **Telegram Mini App Form (`/webapp/activate`)**:
  - Thu thập và chuẩn hóa đúng định dạng: `customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl`.
  - Hỗ trợ **Ô Dán Nhanh**, **Hướng dẫn từng ô**, và **tính trực tiếp mã OTP 6 số (TOTP)** trên trình duyệt để khách tự kiểm chứng chuỗi 2FA 32 ký tự trước khi gửi.
  - Xác thực `Telegram.WebApp.initData` bằng HMAC-SHA256 và mã hóa chuỗi `Mail | Pass | 2FA` bằng **AES-256-GCM** trước khi lưu PostgreSQL.
- **Trang Hướng Dẫn Toàn Diện (`/webapp/guide`)**:
  - Hướng dẫn 4 bước cầm tay chỉ việc: Kiểm tra Quốc gia Gmail (Việt Nam), Rời Family, Đóng Hồ Sơ Thanh Toán (HSTT), Bật 2FA & lấy mã khóa 32 ký tự kèm công cụ test mã 6 số.
- **Nạp Tiền Tự Động qua PayOS (`/payadd`)**:
  - Gọi API PayOS tạo link & mã VietQR động.
  - Nhận Webhook `POST /api/v1/webhooks/payos`, xác thực chữ ký **HMAC-SHA256**, chống cộng tiền 2 lần (Idempotent), tự động cộng ví & trả thưởng Referral lần nạp đầu.
- **100% API Chuẩn hóa (`ApiResponse<T>`)**:
  - Mọi endpoint dưới `/api/v1/*` đều trả về đúng 1 cấu trúc JSON `{ success, code, message, data, meta, error }`.

---

## 2. Hướng Dẫn Kết Nối Bot Telegram & PayOS Từng Bước

### Bước 1: Tạo Bot Telegram qua `@BotFather`
1. Mở Telegram, tìm **`@BotFather`** và gõ `/newbot`.
2. Đặt tên hiển thị (ví dụ: `Gemini Pro Activation Bot`) và username kết thúc bằng `bot` (ví dụ: `gemini_pro_shop_bot`).
3. Copy đoạn **HTTP API Token** (dạng `1234567890:AAF...`) $\rightarrow$ điền vào biến `TELEGRAM_BOT_TOKEN`.
4. Lấy **Telegram User ID của Admin**: nhắn `/start` cho **`@userinfobot`** để lấy dãy số ID $\rightarrow$ điền vào biến `ADMIN_TELEGRAM_ID` (để Bot gửi đơn mới kèm chuỗi `email | pass | 2fa` thẳng về Telegram của bạn).

### Bước 2: Lấy Khóa Kết Nối PayOS (`https://my.payos.vn`)
1. Đăng nhập trang quản trị **PayOS** (`https://my.payos.vn`), liên kết tài khoản ngân hàng nhận tiền (VietQR).
2. Vào mục **Kênh thanh toán (Payment Channels)** $\rightarrow$ Tạo kênh thanh toán mới.
3. Copy 3 khóa sau vào file `.env` (hoặc Render Environment):
   - `PAYOS_CLIENT_ID`
   - `PAYOS_API_KEY`
   - `PAYOS_CHECKSUM_KEY`
4. Sau khi deploy lên Render (hoặc mở ngrok local), dán Webhook URL sau vào mục **Webhook URL** trên PayOS:
   ```text
   https://<domain-render-cua-ban>.onrender.com/api/v1/webhooks/payos
   ```

### Bước 3: Sinh Khóa Mã Hóa AES-256-GCM (`CREDENTIALS_MASTER_KEY`)
Khóa phải dài đúng **64 ký tự hex (= 32 bytes)**. Trên PowerShell / Terminal:
```bash
# Linux / macOS / Git Bash:
openssl rand -hex 32

# Hoặc trên Windows PowerShell:
-join ((1..32) | ForEach-Object { "{0:x2}" -f (Get-Random -Minimum 0 -Maximum 256) })
```

### Bước 4: Chạy Thử Trên Máy Local (Dev Mode)
1. Copy `.env.example` thành `.env` và điền đầy đủ thông tin ở Bước 1–3 (`BOT_MODE=polling`).
2. Đảm bảo đã tạo database PostgreSQL `bot_store_telegram` (migration sẽ **tự động chạy** khi khởi động app).
3. Chạy lệnh:
   ```bash
   cargo run
   ```
4. *(Tùy chọn test Mini App Form ở local)*: Telegram yêu cầu URL của Mini App phải là **HTTPS**. Chạy `ngrok http 10000`, copy URL `https://xxxx.ngrok-free.app` điền vào `PUBLIC_BASE_URL` trong `.env` rồi khởi động lại `cargo run`.

### Bước 5: Deploy Lên Render.com (Production — 1 Click Blueprint)
1. Push mã nguồn lên GitHub repository của bạn.
2. Vào **Render.com** $\rightarrow$ **New** $\rightarrow$ **Blueprint** $\rightarrow$ Chọn repository `BotStoreTelegram`.
3. Render tự đọc file `render.yaml`, tự tạo cả **Render PostgreSQL** lẫn **Rust Docker Web Service**.
4. Nhập các biến môi trường bí mật (`PUBLIC_BASE_URL`, `TELEGRAM_BOT_TOKEN`, `ADMIN_TELEGRAM_ID`, `CREDENTIALS_MASTER_KEY`, `PAYOS_CLIENT_ID`, `PAYOS_API_KEY`, `PAYOS_CHECKSUM_KEY`).
5. Khi deploy xong, hệ thống tự động:
   - Chạy migration tạo toàn bộ bảng & seed gói Gemini Pro (`40.000 đ`).
   - Tự động đăng ký **Telegram Webhook** về `https://<domain>.onrender.com/api/v1/webhooks/telegram` kèm `secret_token`.
   - Sẵn sàng nhận nạp tiền PayOS & mở Telegram Mini App Form!

---

## 3. Admin REST API (Xử Lý Đơn & Hoàn Tiền Đơn Lỗi)

Tất cả Admin API yêu cầu header: `X-Admin-Api-Key: <ADMIN_API_KEY>`

- **Xem danh sách đơn đang chờ (đã giải mã `email | pass | 2fa`)**:
  `GET /api/v1/admin/orders?status=PENDING&page=1&per_page=20`
- **Đánh dấu đơn đã kích hoạt thành công (tự động báo khách qua Telegram)**:
  `POST /api/v1/admin/orders/{order_id}/complete`
  Body: `{ "admin_note": "Đã kích hoạt Gemini Pro 3 tháng thành công!" }`
- **Hoàn tiền 100% cho đơn lỗi (tự động cộng lại ví & báo lý do cho khách)**:
  `POST /api/v1/admin/orders/{order_id}/refund`
  Body: `{ "reason": "Gmail chưa đóng Hồ sơ thanh toán (HSTT)" }`
- **Cập nhật giá hoặc tồn kho gói Gemini Pro**:
  `PUT /api/v1/admin/plans/{plan_id}`
  Body: `{ "price_vnd": 40000, "available_stock": 500 }`
- **Cộng tiền ví thủ công (để test hoặc hỗ trợ khách CK tay)**:
  `POST /api/v1/admin/wallets/{telegram_id}/credit`
  Body: `{ "amount_vnd": 40000, "description": "Test nạp ví" }`
