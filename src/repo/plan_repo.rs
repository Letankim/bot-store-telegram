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
         WHERE id = $1 AND is_active = TRUE AND available_stock > 0
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

pub async fn update_plan_stock_and_price(
    pool: &PgPool,
    plan_id: Uuid,
    price_vnd: Option<i64>,
    available_stock: Option<i32>,
    is_active: Option<bool>,
) -> Result<GeminiPlan, AppError> {
    let updated = sqlx::query_as::<_, GeminiPlan>(
        "UPDATE gemini_plans
         SET price_vnd = COALESCE($1, price_vnd),
             available_stock = COALESCE($2, available_stock),
             is_active = COALESCE($3, is_active),
             updated_at = NOW()
         WHERE id = $4
         RETURNING *",
    )
    .bind(price_vnd)
    .bind(available_stock)
    .bind(is_active)
    .bind(plan_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Không tìm thấy gói Gemini Pro".to_string()))?;

    Ok(updated)
}
