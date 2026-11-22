use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{domain::gemini_plan::GeminiPlan, error::AppError};

pub async fn list_active_plans(pool: &PgPool) -> Result<Vec<GeminiPlan>, AppError> {
    let plans = sqlx::query_as::<_, GeminiPlan>(
        "SELECT * FROM gemini_plans WHERE is_active = TRUE ORDER BY display_order ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(plans)
}

pub async fn list_all_plans(pool: &PgPool) -> Result<Vec<GeminiPlan>, AppError> {
    let plans = sqlx::query_as::<_, GeminiPlan>(
        "SELECT * FROM gemini_plans ORDER BY display_order ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(plans)
}

pub async fn find_by_id(pool: &PgPool, plan_id: Uuid) -> Result<Option<GeminiPlan>, AppError> {
    let plan = sqlx::query_as::<_, GeminiPlan>("SELECT * FROM gemini_plans WHERE id = $1")
        .bind(plan_id)
        .fetch_optional(pool)
        .await?;
    Ok(plan)
}

pub async fn find_default_active(pool: &PgPool) -> Result<Option<GeminiPlan>, AppError> {
    let plan = sqlx::query_as::<_, GeminiPlan>(
        "SELECT * FROM gemini_plans WHERE is_active = TRUE ORDER BY display_order ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(plan)
}

pub async fn decrement_stock_and_increment_sold(
    tx: &mut Transaction<'_, Postgres>,
    plan_id: Uuid,
) -> Result<GeminiPlan, AppError> {
    let plan = sqlx::query_as::<_, GeminiPlan>(
        "UPDATE gemini_plans
         SET available_stock = available_stock - 1,
             sold_count = sold_count + 1,
             updated_at = NOW()
         WHERE id = $1 AND is_active = TRUE AND is_maintenance = FALSE AND available_stock > 0
         RETURNING *",
    )
    .bind(plan_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AppError::OutOfStock)?;

    Ok(plan)
}

pub async fn restore_stock_on_refund(
    tx: &mut Transaction<'_, Postgres>,
    plan_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE gemini_plans
         SET available_stock = available_stock + 1,
             sold_count = GREATEST(sold_count - 1, 0),
             updated_at = NOW()
         WHERE id = $1",
    )
    .bind(plan_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_plan(
    pool: &PgPool,
    code: &str,
    name: &str,
    duration_months: i32,
    price_vnd: i64,
    available_stock: i32,
    description_note: &str,
    api_mode: &str,
    display_order: i32,
) -> Result<GeminiPlan, AppError> {
    let created = sqlx::query_as::<_, GeminiPlan>(
        "INSERT INTO gemini_plans (
            code, name, duration_months, price_vnd, available_stock, sold_count,
            description_note, api_mode, is_active, display_order
         ) VALUES ($1, $2, $3, $4, $5, 0, $6, $7, TRUE, $8)
         RETURNING *",
    )
    .bind(code.trim().to_uppercase())
    .bind(name.trim())
    .bind(duration_months)
    .bind(price_vnd)
    .bind(available_stock)
    .bind(description_note.trim())
    .bind(api_mode)
    .bind(display_order)
    .fetch_one(pool)
    .await?;

    Ok(created)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_plan_full(
    pool: &PgPool,
    plan_id: Uuid,
    name: Option<&str>,
    duration_months: Option<i32>,
    price_vnd: Option<i64>,
    available_stock: Option<i32>,
    sold_count: Option<i32>,
    description_note: Option<&str>,
    api_mode: Option<&str>,
    display_order: Option<i32>,
    is_active: Option<bool>,
    is_maintenance: Option<bool>,
    maintenance_note: Option<&str>,
) -> Result<GeminiPlan, AppError> {
    let updated = sqlx::query_as::<_, GeminiPlan>(
        "UPDATE gemini_plans
         SET name = COALESCE($1, name),
             duration_months = COALESCE($2, duration_months),
             price_vnd = COALESCE($3, price_vnd),
             available_stock = COALESCE($4, available_stock),
             sold_count = COALESCE($5, sold_count),
             description_note = COALESCE($6, description_note),
             api_mode = COALESCE($7, api_mode),
             display_order = COALESCE($8, display_order),
             is_active = COALESCE($9, is_active),
             is_maintenance = COALESCE($10, is_maintenance),
             maintenance_note = COALESCE($11, maintenance_note),
             updated_at = NOW()
         WHERE id = $12
         RETURNING *",
    )
    .bind(name.map(str::trim))
    .bind(duration_months)
    .bind(price_vnd)
    .bind(available_stock)
    .bind(sold_count)
    .bind(description_note.map(str::trim))
    .bind(api_mode)
    .bind(display_order)
    .bind(is_active)
    .bind(is_maintenance)
    .bind(maintenance_note.map(str::trim))
    .bind(plan_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Không tìm thấy gói Gemini Pro".to_string()))?;

    Ok(updated)
}
