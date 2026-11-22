use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

use crate::{
    crypto::{compute_admin_session_token, constant_time_eq_str, sha256_hex},
    domain::user::User,
    error::AppError,
    repo::user_repo,
    state::SharedState,
};

pub const ADMIN_COOKIE_NAME: &str = "admin_session";
pub const WEB_SESSION_COOKIE_NAME: &str = "web_session";

#[derive(Debug, Clone)]
pub struct WebAuthUser(pub User);

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

/// Attach hardened HTTP security headers to every response (keeping `/webapp/*` frameable by Telegram).
pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let is_telegram_webapp = req.uri().path().starts_with("/webapp/");
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    if !is_telegram_webapp {
        headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    }
    resp
}

/// Protect `/api/v1/web/*` authenticated endpoints using server-side HttpOnly `web_session` cookie
/// + `X-Requested-With: XMLHttpRequest` CSRF guard on state-changing requests.
pub async fn require_web_session(
    State(state): State<SharedState>,
    mut req: Request,
    next: Next,
) -> Result<Response, Response> {
    let req_id = req
        .extensions()
        .get::<RequestContext>()
        .map(|c| c.request_id.clone())
        .unwrap_or_else(|| format!("req_{}", Uuid::new_v4().simple()));

    // CSRF defense-in-depth on state-changing methods
    if matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
        let has_xhr = req
            .headers()
            .get("x-requested-with")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("XMLHttpRequest"))
            .unwrap_or(false);
        if !has_xhr {
            return Err(
                AppError::Forbidden("Thiếu header bảo vệ CSRF hợp lệ.".to_string())
                    .into_api_response(req_id),
            );
        }
    }

    let Some(raw_token) = extract_cookie(req.headers(), WEB_SESSION_COOKIE_NAME) else {
        return Err(
            AppError::Unauthorized("Vui lòng đăng nhập tài khoản để tiếp tục.".to_string())
                .into_api_response(req_id),
        );
    };

    let token_hash = sha256_hex(&raw_token);
    let user = match user_repo::find_user_by_session_hash(&state.pool, &token_hash).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return Err(
                AppError::Unauthorized("Phiên đăng nhập đã hết hạn, vui lòng đăng nhập lại.".to_string())
                    .into_api_response(req_id),
            );
        }
        Err(err) => return Err(err.into_api_response(req_id)),
    };

    if user.is_banned {
        let _ = user_repo::delete_all_user_sessions(&state.pool, user.id).await;
        return Err(
            AppError::Forbidden("Tài khoản của bạn đã bị khóa.".to_string())
                .into_api_response(req_id),
        );
    }

    req.extensions_mut().insert(WebAuthUser(user));
    Ok(next.run(req).await)
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
