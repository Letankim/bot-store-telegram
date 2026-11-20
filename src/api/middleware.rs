use axum::{
    extract::{Request, State},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

use crate::{
    crypto::{compute_admin_session_token, constant_time_eq_str},
    error::AppError,
    state::SharedState,
};

pub const ADMIN_COOKIE_NAME: &str = "admin_session";

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub request_id: String,
}

/// Extract a named cookie value from the HTTP `Cookie` header.
pub fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        if let Some((k, v)) = trimmed.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Check if HTTP headers carry a valid `admin_session` HttpOnly cookie or `X-Admin-Api-Key`.
pub fn is_admin_authenticated(headers: &HeaderMap, admin_api_key: &str) -> bool {
    // 1. Check X-Admin-Api-Key header (CLI / API clients)
    if let Some(provided) = headers.get("x-admin-api-key").and_then(|v| v.to_str().ok()) {
        if constant_time_eq_str(provided, admin_api_key) {
            return true;
        }
    }

    // 2. Check HttpOnly admin_session cookie (Browser Admin Dashboard)
    if let Some(cookie_val) = extract_cookie(headers, ADMIN_COOKIE_NAME) {
        if let Ok(expected_token) = compute_admin_session_token(admin_api_key) {
            if constant_time_eq_str(&cookie_val, &expected_token) {
                return true;
            }
        }
    }

    false
}

/// Attach a unique `request_id` to every incoming HTTP request for `ApiResponse.meta`.
pub async fn request_context_middleware(mut req: Request, next: Next) -> Response {
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("req_{}", &Uuid::new_v4().simple().to_string()[..12]));

    req.extensions_mut().insert(RequestContext {
        request_id: request_id.clone(),
    });

    let mut resp = next.run(req).await;
    if let Ok(val) = request_id.parse() {
        resp.headers_mut().insert("x-request-id", val);
    }
    resp
}

/// Protect `/api/v1/admin/*` endpoints with either `X-Admin-Api-Key` or HttpOnly `admin_session` cookie.
pub async fn require_admin_api_key(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let req_id = req
        .extensions()
        .get::<RequestContext>()
        .map(|c| c.request_id.clone())
        .unwrap_or_else(|| format!("req_{}", Uuid::new_v4().simple()));

    if !is_admin_authenticated(req.headers(), &state.config.admin_api_key) {
        return Err(
            AppError::Unauthorized("Chưa đăng nhập quản trị hoặc sai ADMIN_API_KEY.".to_string())
                .into_api_response(req_id),
        );
    }

    Ok(next.run(req).await)
}
