use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

use crate::{crypto::constant_time_eq_str, error::AppError, state::SharedState};

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub request_id: String,
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

/// Protect `/api/v1/admin/*` endpoints with `X-Admin-Api-Key` constant-time verification.
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

    let provided = req
        .headers()
        .get("x-admin-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_eq_str(provided, &state.config.admin_api_key) {
        return Err(
            AppError::Unauthorized("Thiếu hoặc sai X-Admin-Api-Key.".to_string())
                .into_api_response(req_id),
        );
    }

    Ok(next.run(req).await)
}
