---
name: db-transaction-ledger
description: >-
  Use this skill when writing or modifying PostgreSQL database transactions with SQLx for PayOS wallet top-ups, Gemini Pro activation pre-checks and order submission, referral first-deposit rewards, or order refunds.
---

# Database Transaction & Ledger Skill (`db-transaction-ledger`)

Quy trình bắt buộc khi triển khai các nghiệp vụ liên quan đến ví tiền (`VND`), đặt đơn kích hoạt Gemini Pro (`Mail | Pass | 2FA`), nạp tiền qua PayOS và hoàn tiền đơn lỗi trong `src/repo/` và `src/services/`.

---

## 1. Quy trình 2 Bước: Kiểm tra Số dư & Submit Form Kích hoạt Gemini Pro

### Bước A: Pre-check trước khi mở Form Mini App (Khi bấm nút "Kích hoạt ngay" trên Telegram)
Trước khi hiển thị nút mở Form Mini App cho khách nhập `Mail | Pass | 2FA`:
1. Truy vấn thông tin gói `gemini_plans` (`is_active = true`). Kiểm tra `available_stock > 0`. Nếu hết slot $\rightarrow$ báo hết slot.
2. Truy vấn số dư ví của khách trong bảng `wallets`.
3. Kiểm tra `wallet.balance_vnd >= plan.price_vnd` (ví dụ `>= 40000`).
   - Nếu **không đủ tiền**: Bot thông báo số tiền còn thiếu và hiển thị ngay nút **Nạp tiền tự động qua PayOS (VietQR)** đúng bằng số tiền cần nạp.
   - Nếu **đủ tiền**: Bot trả về nút **Telegram WebApp (`WebAppInfo`)** mở Form điền thông tin `Email | Mật khẩu | Mã 2FA` + Nút mở **Trang Hướng Dẫn Chi Tiết**.

### Bước B: Atomic Transaction khi khách Submit Form Mini App (`POST /api/v1/webapp/activations`)
Khi khách điền xong form và bấm gửi, toàn bộ các thao tác sau phải nằm trong **cùng một PostgreSQL Transaction (`sqlx::Transaction<'_, Postgres>`)**:

1. **Khóa Ví người mua (`FOR UPDATE`)**:
   ```sql
   SELECT id, user_id, balance_vnd
   FROM wallets
   WHERE user_id = $1
   FOR UPDATE;
   ```
   - Kiểm tra lại `balance_vnd >= plan.price_vnd` (chống trường hợp người dùng mở 2 form cùng lúc). Nếu không đủ, rollback và trả về `AppError::InsufficientBalance`.

2. **Khóa & Trừ Tồn kho Gói Gemini Pro (`FOR UPDATE`)**:
   ```sql
   UPDATE gemini_plans
   SET available_stock = available_stock - 1,
       sold_count = sold_count + 1,
       updated_at = NOW()
   WHERE id = $1 AND is_active = true AND available_stock > 0
   RETURNING id, name, price_vnd;
   ```
   - Nếu không có dòng nào được cập nhật (`0 rows`), rollback và trả về `AppError::OutOfStock`.

3. **Trừ tiền Ví & Ghi Sổ cái (`wallet_transactions`)**:
   ```sql
   UPDATE wallets
   SET balance_vnd = balance_vnd - $1, updated_at = NOW()
   WHERE id = $2;

   INSERT INTO wallet_transactions (
       id, wallet_id, tx_type, amount_vnd, balance_before, balance_after, idempotency_key, description
   ) VALUES ($1, $2, 'GEMINI_ACTIVATION', -$3, $4, $5, $6, $7);
   ```

4. **Mã hóa Chuỗi `email | pass | 2fa` và Lưu Đơn Kích hoạt (`activation_orders`)**:
   - Chuẩn hóa chuỗi:
     `let formatted = format!("{} | {} | {}", email.trim(), password.trim(), normalized_2fa);`
   - Mã hóa bằng AES-256-GCM $\rightarrow$ `(ciphertext, nonce)`.
   - `INSERT INTO activation_orders (...) VALUES (...)` với trạng thái `PENDING` (chờ xử lý trong 1-6h).
   - `tx.commit().await?` rồi gửi tin nhắn xác nhận qua Telegram cho khách & thông báo cho Admin.

---

## 2. Quy trình Hoàn tiền Đơn Lỗi (`Only Refund for Defective Orders`)

Đúng theo mô tả trên Bot (`‼️ Chỉ hỗ trợ refund với những đơn hàng lỗi`):
Khi Admin đánh dấu đơn kích hoạt là lỗi (`FAILED_REFUNDED`) qua Admin API hoặc lệnh Admin:
1. Mở Transaction `tx`.
2. Khóa đơn hàng `SELECT * FROM activation_orders WHERE id = $1 FOR UPDATE`. Chỉ cho phép hoàn tiền nếu `status == 'PENDING' || status == 'PROCESSING'`.
3. Khóa ví khách hàng `SELECT * FROM wallets WHERE user_id = $1 FOR UPDATE`.
4. Cộng lại `price_paid_vnd` vào `wallets.balance_vnd`.
5. Ghi sổ cái `wallet_transactions` với `tx_type = 'ORDER_REFUND'`, `idempotency_key = format!("refund_{}", order.id)`.
6. Cập nhật `activation_orders.status = 'REFUNDED'`, hoàn lại `available_stock + 1` và `sold_count - 1` cho `gemini_plans`.
7. `tx.commit().await?` và gửi thông báo Telegram cho khách biết tiền đã được hoàn về ví kèm lý do lỗi (ví dụ: Gmail chưa đóng HSTT, sai mật khẩu, hoặc Gmail đang ở trong Family khác).
