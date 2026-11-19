# Rule 03: Database Design, Ledger Integrity & Security Protocols

## 1. Quy tắc Thiết kế Cơ sở dữ liệu PostgreSQL
1. **Kiểu dữ liệu bắt buộc**:
   - Tiền tệ (`VND`): Dùng `BIGINT NOT NULL DEFAULT 0` (lưu số đồng nguyên, ví dụ `40000` tương ứng `40.000 đ`, ràng buộc `CHECK (balance_vnd >= 0)` trên bảng `wallets`).
   - Thời gian: `TIMESTAMPTZ NOT NULL DEFAULT NOW()`.
   - Trạng thái: Dùng `VARCHAR(32)` kết hợp `CHECK (...)`.
2. **Sổ cái Bất biến (Append-Only Ledger)**:
   - Bảng `wallet_transactions` là **Append-Only** (chỉ `INSERT`, cấm `UPDATE` hoặc `DELETE`).
   - Mỗi dòng trong `wallet_transactions` phải lưu rõ `balance_before`, `amount_vnd` (dương hoặc âm), `balance_after`, `tx_type` (`PAYOS_TOPUP`, `GEMINI_ACTIVATION`, `ORDER_REFUND`, `REFERRAL_REWARD`, `ADMIN_ADJUST`), và `idempotency_key` (`UNIQUE` để chống cộng tiền 2 lần khi PayOS gọi lại webhook).
3. **Chuẩn hóa Lưu trữ Đơn Kích hoạt Gemini Pro (`activation_orders`)**:
   - Dữ liệu người dùng nhập từ Form Mini App bao gồm:
     - `gmail`: Địa chỉ email (ví dụ `customer1@gmail.com`)
     - `gmail_password`: Mật khẩu tài khoản
     - `two_fa_secret`: Mã khóa thiết lập 2FA (ví dụ `ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl`)
   - Hệ thống chuẩn hóa về định dạng chuẩn:
     `customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl`
   - Toàn bộ chuỗi chuẩn hóa này **bắt buộc được mã hóa AES-256-GCM** và lưu vào cột `credentials_encrypted BYTEA` + `nonce BYTEA`. Chỉ lưu `gmail_masked` (ví dụ `cus***1@gmail.com`) ở dạng văn bản rõ để hiển thị lịch sử đơn hàng cho khách.

## 2. Quy tắc Bảo mật Nhiều Lớp (Defense-in-Depth)
1. **Mã hóa Thông tin Tài khoản Khách hàng (`AES-256-GCM`)**:
   - Sử dụng thuật toán **AES-256-GCM** (crate `aes-gcm`) với khóa 32-byte đọc từ biến môi trường `CREDENTIALS_MASTER_KEY`.
   - Mỗi đơn kích hoạt (`activation_order`) bắt buộc sinh một `nonce` ngẫu nhiên 96-bit (12 bytes) riêng biệt từ `OsRng`.
2. **Xác thực Telegram Mini App (`Telegram.WebApp.initData`)**:
   - Khi người dùng bấm nút mở Form Mini App để điền `Mail | Pass | 2FA`, trang WebApp gửi kèm `initData` trong header `X-Telegram-Init-Data`.
   - Server phải xác thực chữ ký HMAC-SHA256 của `initData`:
     1. Tạo `secret_key = HMAC_SHA256(key="WebAppData", msg=TELEGRAM_BOT_TOKEN)`.
     2. Sắp xếp các cặp `key=value` (trừ `hash`) theo thứ tự bảng chữ cái, nối bằng `\n` tạo thành `data_check_string`.
     3. Kiểm tra `HEX(HMAC_SHA256(key=secret_key, msg=data_check_string)) == hash` bằng `subtle::ConstantTimeEq`.
     4. Kiểm tra `auth_date` không quá 1 giờ (`3600s`) để chống Replay Attack.
3. **Xác thực PayOS Webhook (`HMAC-SHA256`)**:
   - Mọi request tới `POST /api/v1/webhooks/payos` phải được kiểm tra chữ ký `signature` dựa trên `PAYOS_CHECKSUM_KEY` bằng thuật toán sắp xếp các key trong object `data` theo thứ tự alphabet (`key1=val1&key2=val2...`) trước khi cộng tiền vào ví.
4. **Bảo vệ Admin API**:
   - Tất cả các route dưới `/api/v1/admin/*` (như lấy danh sách đơn chờ kích hoạt kèm chuỗi `email | pass | 2fa` đã giải mã, hoặc bấm hoàn tiền đơn lỗi) phải được bảo vệ bởi Middleware xác thực JWT / `X-Admin-Api-Key` và Rate Limit.
