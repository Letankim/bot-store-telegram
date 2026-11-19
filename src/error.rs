use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;
use uuid::Uuid;

use crate::response::{ApiErrorDetail, ApiResponse};

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Dữ liệu đầu vào không hợp lệ: {0}")]
    Validation(String),

    #[error("Xác thực thất bại: {0}")]
    Unauthorized(String),

    #[error("Bạn không có quyền thực hiện thao tác này: {0}")]
    Forbidden(String),

    #[error("Không tìm thấy dữ liệu: {0}")]
    NotFound(String),

    #[error("Xung đột dữ liệu hoặc giao dịch trùng lặp: {0}")]
    Conflict(String),

    #[error("Số dư ví không đủ. Cần {required} đ nhưng hiện có {available} đ.")]
    InsufficientBalance { required: i64, available: i64 },

    #[error("Gói dịch vụ hiện đã hết tồn kho.")]
    OutOfStock,

    #[error("Lỗi kết nối cổng thanh toán PayOS: {0}")]
    PaymentGateway(String),

    #[error("Database error")]
    Database(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            AppError::Validation(_) => (StatusCode::BAD_REQUEST, "ERR_VALIDATION_FAILED"),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "ERR_UNAUTHORIZED"),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "ERR_FORBIDDEN"),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "ERR_NOT_FOUND"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "ERR_CONFLICT"),
            AppError::InsufficientBalance { .. } => {
                (StatusCode::UNPROCESSABLE_ENTITY, "ERR_INSUFFICIENT_BALANCE")
            }
            AppError::OutOfStock => (StatusCode::UNPROCESSABLE_ENTITY, "ERR_OUT_OF_STOCK"),
            AppError::PaymentGateway(_) => (StatusCode::BAD_GATEWAY, "ERR_PAYMENT_GATEWAY"),
            AppError::Config(_) | AppError::Database(_) | AppError::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "ERR_INTERNAL_SERVER")
            }
        }
    }

    pub fn into_api_response(self, request_id: String) -> Response {
        let (status, code) = self.status_and_code();

        let (public_message, details) = match &self {
            AppError::Database(err) => {
                tracing::error!(error = ?err, request_id = %request_id, "Database error");
                (
                    "Lỗi hệ thống cơ sở dữ liệu. Vui lòng thử lại sau.".to_string(),
                    None,
                )
            }
            AppError::Internal(err) => {
                tracing::error!(error = %err, request_id = %request_id, "Internal server error");
                ("Lỗi hệ thống nội bộ. Vui lòng thử lại sau.".to_string(), None)
            }
            AppError::Config(err) => {
                tracing::error!(error = %err, request_id = %request_id, "Config error");
                ("Lỗi cấu hình máy chủ.".to_string(), None)
            }
            AppError::InsufficientBalance {
                required,
                available,
            } => (
                "Số dư ví không đủ để kích hoạt gói Gemini Pro này.".to_string(),
                Some(format!(
                    "Cần {} đ nhưng số dư hiện tại là {} đ.",
                    required, available
                )),
            ),
            other => (other.to_string(), None),
        };

        let body = ApiResponse::<serde_json::Value>::error(
            status,
            code,
            public_message,
            request_id,
            ApiErrorDetail {
                details,
                fields: None,
            },
        );
        body.into_response()
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let fallback_req_id = format!("req_{}", Uuid::new_v4().simple());
        self.into_api_response(fallback_req_id)
    }
}
