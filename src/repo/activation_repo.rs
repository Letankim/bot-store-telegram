use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{domain::activation_order::ActivationOrder, error::AppError};

pub async fn create_order(
    tx: &mut Transaction<'_, Postgres>,
    order_code: &str,
    user_id: Uuid,
    plan_id: Uuid,
    price_paid_vnd: i64,
    gmail_masked: &str,
    credentials_encrypted: &[u8],
    nonce: &[u8],
) -> Result<ActivationOrder, AppError> {
    let order = sqlx::query_as::<_, ActivationOrder>(
        "INSERT INTO activation_orders (
            order_code, user_id, plan_id, price_paid_vnd, gmail_masked,
            credentials_encrypted, nonce, status
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'PENDING')
         RETURNING *",
    )
    .bind(order_code)
    .bind(user_id)
    .bind(plan_id)
    .bind(price_paid_vnd)
    .bind(gmail_masked)
    .bind(credentials_encrypted)
    .bind(nonce)
    .fetch_one(&mut **tx)
    .await?;

    Ok(order)
}

pub async fn lock_by_id(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
) -> Result<ActivationOrder, AppError> {
    sqlx::query_as::<_, ActivationOrder>("SELECT * FROM activation_orders WHERE id = $1 FOR UPDATE")
        .bind(order_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::NotFound("Không tìm thấy đơn kích hoạt".to_string()))
}

pub async fn list_by_user(
    pool: &PgPool,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<ActivationOrder>, AppError> {
    let rows = sqlx::query_as::<_, ActivationOrder>(
        "SELECT * FROM activation_orders
         WHERE user_id = $1
         ORDER BY created_at DESC
         LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_for_admin(
    pool: &PgPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ActivationOrder>, i64), AppError> {
    let (rows, total) = if let Some(st) = status {
        let rows = sqlx::query_as::<_, ActivationOrder>(
            "SELECT * FROM activation_orders
             WHERE status = $1
             ORDER BY created_at ASC
             LIMIT $2 OFFSET $3",
        )
        .bind(st)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM activation_orders WHERE status = $1")
                .bind(st)
                .fetch_one(pool)
                .await?;
        (rows, total.0)
    } else {
        let rows = sqlx::query_as::<_, ActivationOrder>(
            "SELECT * FROM activation_orders
             ORDER BY created_at DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM activation_orders")
            .fetch_one(pool)
            .await?;
        (rows, total.0)
    };

    Ok((rows, total))
}

pub async fn mark_completed(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    admin_note: Option<&str>,
) -> Result<ActivationOrder, AppError> {
    let updated = sqlx::query_as::<_, ActivationOrder>(
        "UPDATE activation_orders
         SET status = 'COMPLETED',
             admin_note = $1,
             completed_at = NOW(),
             updated_at = NOW()
         WHERE id = $2
         RETURNING *",
    )
    .bind(admin_note)
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(updated)
}

pub async fn mark_refunded(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    admin_note: &str,
) -> Result<ActivationOrder, AppError> {
    let updated = sqlx::query_as::<_, ActivationOrder>(
        "UPDATE activation_orders
         SET status = 'REFUNDED',
             admin_note = $1,
             updated_at = NOW()
         WHERE id = $2
         RETURNING *",
    )
    .bind(admin_note)
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(updated)
}

pub async fn find_by_id(
    pool: &PgPool,
    order_id: Uuid,
) -> Result<Option<ActivationOrder>, AppError> {
    let order = sqlx::query_as::<_, ActivationOrder>(
        "SELECT * FROM activation_orders WHERE id = $1",
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?;
    Ok(order)
}

/// Fetch the oldest PENDING order eligible for automatic upstream Pixel API push (FIFO queue).
/// Excludes manual products and products whose `form_mode != 'mail_pass_2fa'`.
pub async fn find_next_queued_pending(
    pool: &PgPool,
) -> Result<Option<ActivationOrder>, AppError> {
    let order = sqlx::query_as::<_, ActivationOrder>(
        "SELECT o.* FROM activation_orders o
         JOIN gemini_plans p ON p.id = o.plan_id
         WHERE o.status = 'PENDING'
           AND o.external_job_id IS NULL
           AND (o.external_status IS NULL OR o.external_status = 'queued')
           AND p.form_mode = 'mail_pass_2fa'
           AND p.api_mode <> 'manual'
         ORDER BY o.created_at ASC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(order)
}

pub async fn mark_submit_failed(
    pool: &PgPool,
    order_id: Uuid,
    reason: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE activation_orders
         SET external_status = 'submit_failed',
             admin_note = $1,
             updated_at = NOW()
         WHERE id = $2",
    )
    .bind(format!("Lỗi đẩy API tự động: {reason}"))
    .bind(order_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_external_job_started(
    pool: &PgPool,
    order_id: Uuid,
    job_id: i64,
    mode: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE activation_orders
         SET status = 'PROCESSING',
             external_job_id = $1,
             external_status = 'queued',
             admin_note = COALESCE(admin_note, $2),
             updated_at = NOW()
         WHERE id = $3",
    )
    .bind(job_id)
    .bind(format!("Đã đẩy sang API tự động ({mode}, Job #{job_id})"))
    .bind(order_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_external_progress(
    pool: &PgPool,
    order_id: Uuid,
    external_status: &str,
    external_step: Option<&str>,
    external_result: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE activation_orders
         SET external_status = $1,
             external_step = COALESCE($2, external_step),
             external_result = COALESCE($3, external_result),
             updated_at = NOW()
         WHERE id = $4",
    )
    .bind(external_status)
    .bind(external_step)
    .bind(external_result)
    .bind(order_id)
    .execute(pool)
    .await?;
    Ok(())
}
