---
name: payos-payment-integration
description: >-
  Use this skill when implementing PayOS payment link generation, VietQR display in Telegram, HMAC-SHA256 checksum signing, and PayOS webhook verification in Rust.
---

# PayOS Payment Gateway Integration Skill (`payos-payment-integration`)

Sử dụng skill này khi xây dựng module nạp tiền tự động qua **PayOS** (`src/domain/payos.rs`, `src/services/payos_service.rs`, và `POST /api/v1/webhooks/payos`).

---

## 1. Biến Môi trường Bắt buộc cho PayOS

Trong `src/config.rs`:
- `PAYOS_CLIENT_ID`: Client ID từ trang quản trị PayOS (`x-client-id`).
- `PAYOS_API_KEY`: API Key từ trang quản trị PayOS (`x-api-key`).
- `PAYOS_CHECKSUM_KEY`: Checksum Key dùng để ký và xác thực HMAC-SHA256.
- `PAYOS_BASE_URL`: Mặc định `https://api-merchant.payos.vn`.

---

## 2. Công thức Ký HMAC-SHA256 Khi Tạo Link Thanh Toán (`POST /v2/payment-requests`)

Theo tài liệu chính thức của PayOS, chữ ký `signature` khi tạo yêu cầu thanh toán được tính bằng **HMAC-SHA256** với khóa `PAYOS_CHECKSUM_KEY` trên chuỗi dữ liệu được sắp xếp theo đúng thứ tự 5 trường alphabet:

```text
amount={amount}&cancelUrl={cancelUrl}&description={description}&orderCode={orderCode}&returnUrl={returnUrl}
```

### Cài đặt Chuẩn trong Rust (`src/crypto.rs`):
```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn sign_payos_create_payment(
    amount: i64,
    cancel_url: &str,
    description: &str,
    order_code: i64,
    return_url: &str,
    checksum_key: &str,
) -> Result<String, AppError> {
    let data = format!(
        "amount={}&cancelUrl={}&description={}&orderCode={}&returnUrl={}",
        amount, cancel_url, description, order_code, return_url
    );
    let mut mac = HmacSha256::new_from_slice(checksum_key.as_bytes())
        .map_err(|_| AppError::Internal("Invalid PayOS checksum key".into()))?;
    mac.update(data.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}
```
- **Lưu ý về `orderCode` và `description` của PayOS**:
  - `orderCode` phải là số nguyên dương (`i64`) tối đa `9007199254740991` (JS `MAX_SAFE_INTEGER`) và là duy nhất cho mỗi giao dịch nạp.
  - `description` tối đa **25 ký tự** (khuyến nghị: `NAP {order_code}` ví dụ `NAP 84920192`, chỉ dùng chữ cái không dấu và số).

---

## 3. Công thức Xác thực Chữ ký Webhook từ PayOS (`POST /api/v1/webhooks/payos`)

Khi khách quét mã VietQR chuyển khoản thành công, PayOS bắn Webhook có dạng:
```json
{
  "code": "00",
  "desc": "success",
  "success": true,
  "data": {
    "orderCode": 84920192,
    "amount": 40000,
    "description": "NAP 84920192",
    "accountNumber": "12345678",
    "reference": "TF230204212323",
    "transactionDateTime": "2026-11-19 18:55:00",
    "currency": "VND",
    "paymentLinkId": "124c33293c43417ab7879e14c8d9eb18",
    "code": "00",
    "desc": "Thành công"
  },
  "signature": "8d8640d802576397a1ce45ebda7f835055768ac7ad2e0bfb77f9b8f12cca4c7f"
}
```

### Thuật toán Xác thực `signature` của PayOS Webhook trong Rust:
1. Lấy `data` dưới dạng `serde_json::Map<String, serde_json::Value>`.
2. Sắp xếp tất cả các key của `data` theo thứ tự bảng chữ cái (`BTreeMap` trong Rust tự động sắp xếp theo alphabet!).
3. Với mỗi `(key, value)`:
   - Nếu `value` là `Null`, chuyển thành chuỗi rỗng `""`.
   - Nếu `value` là `String(s)`, giữ nguyên `s`.
   - Nếu `value` là số hoặc bool, chuyển thành `value.to_string()`.
4. Nối tất cả các cặp `key=value` bằng dấu `&` $\rightarrow$ tính `HMAC_SHA256(checksum_key, query_string)` $\rightarrow$ so sánh với `payload.signature` bằng `subtle::ConstantTimeEq`.

---

## 4. Xử lý Idempotency & Cộng Tiền Tự Động
1. Tìm bản ghi `payos_deposits` theo `order_code = data.orderCode` với `FOR UPDATE`.
2. Nếu `status == 'PAID'` $\rightarrow$ trả về `ApiResponse::ok(..., "Already processed")` ngay lập tức (HTTP 200 để PayOS không retry).
3. Nếu `status == 'PENDING'` và `payload.code == "00"` và `data.code == "00"`:
   - Cập nhật `payos_deposits.status = 'PAID'`, `paid_at = NOW()`, `reference = data.reference`.
   - Cộng `data.amount` vào `wallets.balance_vnd` của user và ghi dòng `wallet_transactions` (`tx_type = 'PAYOS_TOPUP'`).
   - Nếu user này có `referrals.status == 'PENDING_FIRST_DEPOSIT'`, tự động cộng thưởng giới thiệu cho người giới thiệu.
   - Commit transaction và gửi thông báo Telegram tới user:
     `✅ Nạp thành công {amount} đ qua PayOS! Số dư hiện tại: {new_balance} đ. Bạn có thể bấm Kích hoạt Gemini Pro ngay bây giờ.`
