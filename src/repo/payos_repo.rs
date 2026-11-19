use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{domain::payos::PayOsDeposit, error::AppError};

pub async fn create_deposit(
    pool: &PgPool,
    order_code: i64,
    user_id: Uuid,
    amount_vnd: i64,
    description: &str,
    checkout_url: &str,
    qr_code: &str,
    payment_link_id: &str,
) -> Result<PayOsDeposit, AppError> {
    let deposit = sqlx::query_as::<_, PayOsDeposit>(
        "INSERT INTO payos_deposits (
            order_code, user_id, amount_vnd, description,
            checkout_url, qr_code, payment_link_id, status
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'PENDING')
         RETURNING *",
    )
    .bind(order_code)
    .bind(user_id)
    .bind(amount_vnd)
    .bind(description)
    .bind(checkout_url)
    .bind(qr_code)
    .bind(payment_link_id)
    .fetch_one(pool)
    .await?;

    Ok(deposit)
}

pub async fn lock_by_order_code(
    tx: &mut Transaction<'_, Postgres>,
    order_code: i64,
) -> Result<Option<PayOsDeposit>, AppError> {
    let row = sqlx::query_as::<_, PayOsDeposit>(
        "SELECT * FROM payos_deposits WHERE order_code = $1 FOR UPDATE",
    )
    .bind(order_code)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row)
}

pub async fn mark_paid(
    tx: &mut Transaction<'_, Postgres>,
    deposit_id: Uuid,
    reference: Option<&str>,
) -> Result<PayOsDeposit, AppError> {
    let updated = sqlx::query_as::<_, PayOsDeposit>(
        "UPDATE payos_deposits
         SET status = 'PAID',
             reference = $1,
             paid_at = NOW()
         WHERE id = $2
         RETURNING *",
    )
    .bind(reference)
    .bind(deposit_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(updated)
}

pub async fn mark_cancelled(
    tx: &mut Transaction<'_, Postgres>,
    deposit_id: Uuid,
) -> Result<PayOsDeposit, AppError> {
    let updated = sqlx::query_as::<_, PayOsDeposit>(
        "UPDATE payos_deposits
         SET status = 'CANCELLED'
         WHERE id = $1
         RETURNING *",
    )
    .bind(deposit_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(updated)
}
