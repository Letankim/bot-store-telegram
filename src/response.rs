use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    #[serde(skip)]
    pub status: StatusCode,
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

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T, message: impl Into<String>, request_id: String) -> Self {
        Self {
            status: StatusCode::OK,
            success: true,
            code: "SUCCESS",
            message: message.into(),
            data: Some(data),
            meta: ResponseMeta {
                request_id,
                timestamp: Utc::now(),
                pagination: None,
            },
            error: None,
        }
    }

    pub fn created(data: T, message: impl Into<String>, request_id: String) -> Self {
        Self {
            status: StatusCode::CREATED,
            success: true,
            code: "SUCCESS",
            message: message.into(),
            data: Some(data),
            meta: ResponseMeta {
                request_id,
                timestamp: Utc::now(),
                pagination: None,
            },
            error: None,
        }
    }

    pub fn ok_paginated(
        data: T,
        message: impl Into<String>,
        request_id: String,
        pagination: PaginationMeta,
    ) -> Self {
        Self {
            status: StatusCode::OK,
            success: true,
            code: "SUCCESS",
            message: message.into(),
            data: Some(data),
            meta: ResponseMeta {
                request_id,
                timestamp: Utc::now(),
                pagination: Some(pagination),
            },
            error: None,
        }
    }

    pub fn error(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        request_id: String,
        error: ApiErrorDetail,
    ) -> Self {
        Self {
            status,
            success: false,
            code,
            message: message.into(),
            data: None,
            meta: ResponseMeta {
                request_id,
                timestamp: Utc::now(),
                pagination: None,
            },
            error: Some(error),
        }
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}
