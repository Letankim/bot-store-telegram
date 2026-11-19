use std::time::Instant;

use axum::{Extension, extract::State};
use serde::Serialize;

use crate::{
    api::middleware::RequestContext,
    error::AppError,
    response::ApiResponse,
    state::SharedState,
};

#[derive(Debug, Serialize)]
pub struct HealthData {
    pub status: &'static str,
    pub database: &'static str,
    pub db_latency_ms: u128,
    pub version: &'static str,
}

pub async fn health_check(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<HealthData>, AppError> {
    let start = Instant::now();
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    let latency = start.elapsed().as_millis();

    Ok(ApiResponse::ok(
        HealthData {
            status: "healthy",
            database: "connected",
            db_latency_ms: latency,
            version: env!("CARGO_PKG_VERSION"),
        },
        "Hệ thống hoạt động bình thường",
        ctx.request_id,
    ))
}
