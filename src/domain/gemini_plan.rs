use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GeminiPlan {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub duration_months: i32,
    pub price_vnd: i64,
    pub available_stock: i32,
    pub sold_count: i32,
    pub description_note: String,
    pub api_mode: String,
    pub is_active: bool,
    pub is_maintenance: bool,
    pub maintenance_note: String,
    pub display_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
