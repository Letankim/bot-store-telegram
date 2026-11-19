---
name: api-endpoint-scaffold
description: >-
  Use this skill when creating or updating any Axum HTTP REST API endpoint or Webhook handler to guarantee the unified ApiResponse<T> JSON format, request validation, authentication, and error mapping.
---

# Axum API Endpoint Scaffolding Skill (`api-endpoint-scaffold`)

Sử dụng skill này mỗi khi thêm hoặc chỉnh sửa API trong `src/api/` để đảm bảo 100% API tuân thủ định dạng trả về cố định (`ApiResponse<T>`) và bảo mật.

---

## 1. Checklist Khi Tạo Một Endpoint Mới

1. **Định nghĩa DTO Request & Validation**:
   - Dùng `serde::Deserialize` và crate `validator::Validate` để kiểm tra biên dữ liệu (`#[validate(length(min = 1, max = 100))]`, `#[validate(range(min = 1))]`).
2. **Định nghĩa DTO Response**:
   - Dùng `serde::Serialize`. Không bao giờ trả trực tiếp struct của tầng `repo` nếu có chứa các cột nhạy cảm như `credentials_encrypted`, `nonce`, `password_hash`.
3. **Kiểu trả về của Handler**:
   - Mọi Axum handler bắt buộc có chữ ký trả về:
     `Result<ApiResponse<MyOutputDto>, AppError>`
4. **Tích hợp `IntoResponse` cho `ApiResponse<T>` và `AppError`**:
   - Cả nhánh `Ok(ApiResponse<T>)` và nhánh `Err(AppError)` đều phải render ra **cùng một cấu trúc JSON** `{ success, code, message, data, meta, error }`.

---

## 2. Mẫu Handler Chuẩn (Reference Template)

```rust
use axum::{
    extract::{Path, Query, State},
    Extension,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::{
    api::middleware::RequestContext,
    error::AppError,
    response::{ApiResponse, PaginationMeta},
    state::SharedState,
};

#[derive(Debug, Deserialize, Validate)]
pub struct CreateProductReq {
    #[validate(length(min = 2, max = 64))]
    pub code: String,
    #[validate(length(min = 2, max = 128))]
    pub name: String,
    pub price_xu: rust_decimal::Decimal,
    pub warranty_type: String, // "NW" | "W_DAYS" | "FW"
    pub warranty_days: i32,
}

#[derive(Debug, Serialize)]
pub struct ProductRes {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub price_xu: rust_decimal::Decimal,
    pub warranty_badge: String,
    pub available_stock: i64,
}

#[tracing::instrument(skip(state, ctx))]
pub async fn create_product_handler(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    axum::Json(payload): axum::Json<CreateProductReq>,
) -> Result<ApiResponse<ProductRes>, AppError> {
    payload
        .validate()
        .map_err(|e| AppError::Validation(e.to_string()))?;

    let product = state.shop_service.create_product(payload).await?;

    Ok(ApiResponse::created(
        product,
        "Tạo sản phẩm thành công",
        ctx.request_id,
    ))
}
```

---

## 3. Kiểm thử Bắt buộc (Verification Steps)
Sau khi viết xong bất kỳ endpoint nào:
1. Chạy `cargo check` và `cargo clippy -- -D warnings`.
2. Đảm bảo test cả trường hợp `200/201` và trường hợp lỗi `400/401/404/422` đều chứa đầy đủ 6 khóa gốc: `success`, `code`, `message`, `data`, `meta`, `error`.
