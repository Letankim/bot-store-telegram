# Rule 02: Chuẩn hóa Format API Cố định (Unified API Response Envelope)

Mọi HTTP endpoint trong hệ thống (bao gồm Public API, Webhook Response, và Admin API) **bắt buộc** trả về JSON theo đúng một khung cố định duy nhất (`ApiResponse<T>`).

## 1. Cấu trúc JSON Chuẩn

### Khi Thành công (`200 OK`, `201 Created`)
```json
{
  "success": true,
  "code": "SUCCESS",
  "message": "Thao tác thành công",
  "data": {
    "id": "019345a0-7c1e-7000-8000-000000000001",
    "status": "COMPLETED"
  },
  "meta": {
    "request_id": "req_019345a07c1e",
    "timestamp": "2026-11-19T11:36:30Z",
    "pagination": {
      "page": 1,
      "per_page": 20,
      "total_items": 145,
      "total_pages": 8
    }
  },
  "error": null
}
```
*(Ghi chú: trường `pagination` trong `meta` là `null` hoặc lược bỏ bằng `#[serde(skip_serializing_if = "Option::is_none")]` nếu API không phân trang).*

### Khi Thất bại (`4xx`, `5xx`)
```json
{
  "success": false,
  "code": "ERR_INSUFFICIENT_BALANCE",
  "message": "Số dư ví không đủ để thanh toán đơn hàng này.",
  "data": null,
  "meta": {
    "request_id": "req_019345a099ff",
    "timestamp": "2026-11-19T11:37:00Z"
  },
  "error": {
    "details": "Cần 150.00 xu nhưng số dư khả dụng là 45.00 xu.",
    "fields": null
  }
}
```

---

## 2. Cài đặt Rust Bắt buộc trong `src/response.rs` & `src/error.rs`

```rust
use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    pub code: &'static str,
    pub message: String,
    pub data: Option<T>,
    pub meta: ResponseMeta,
    pub error: Option<ApiErrorDetail>,
}

#[derive(Debug, Serialize)]
pub struct ResponseMeta {
    pub request_id: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationMeta>,
}

#[derive(Debug, Serialize)]
pub struct PaginationMeta {
    pub page: u32,
    pub per_page: u32,
    pub total_items: u64,
    pub total_pages: u32,
}

#[derive(Debug, Serialize)]
pub struct ApiErrorDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<serde_json::Value>,
}
```

---

## 3. Bảng Mã Lỗi Chuẩn (`code`)
Agent không được tự bịa mã lỗi ngoài quy ước `ERR_<DOMAIN>_<REASON>`:
- `SUCCESS`: Thành công (`200`, `201`)
- `ERR_VALIDATION_FAILED`: Dữ liệu đầu vào không hợp lệ (`400`)
- `ERR_UNAUTHORIZED`: Thiếu hoặc sai Token / Chữ ký Webhook (`401`)
- `ERR_FORBIDDEN`: Không có quyền truy cập tài nguyên (`403`)
- `ERR_NOT_FOUND`: Không tìm thấy sản phẩm / đơn hàng / người dùng (`404`)
- `ERR_CONFLICT` / `ERR_DUPLICATE_TRANSACTION`: Giao dịch bị trùng lặp idempotency key (`409`)
- `ERR_OUT_OF_STOCK`: Sản phẩm đã hết hàng trong kho (`422`)
- `ERR_INSUFFICIENT_BALANCE`: Số dư ví không đủ (`422`)
- `ERR_WARRANTY_EXPIRED`: Đơn hàng đã hết hạn bảo hành hoặc thuộc diện `NW` (No Warranty) (`422`)
- `ERR_RATE_LIMITED`: Vượt quá giới hạn tần suất gọi API (`429`)
- `ERR_INTERNAL_SERVER`: Lỗi hệ thống nội bộ — tuyệt đối ẩn chi tiết stacktrace/SQL khỏi `message` trả về client (`500`)
