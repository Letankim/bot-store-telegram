use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    domain::wallet::{Wallet, WalletTransaction},
    error::AppError,
};

pub async fn get_by_user_id(pool: &PgPool, user_id: Uuid) -> Result<Wallet, AppError> {
    sqlx::query_as::<_, Wallet>("SELECT * FROM wallets WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Không tìm thấy ví người dùng".to_string()))
}

pub async fn lock_by_user_id(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<Wallet, AppError> {
    sqlx::query_as::<_, Wallet>("SELECT * FROM wallets WHERE user_id = $1 FOR UPDATE")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::NotFound("Không tìm thấy ví người dùng".to_string()))
}

pub async fn credit_wallet(
    tx: &mut Transaction<'_, Postgres>,
    wallet: &Wallet,
    amount_vnd: i64,
    is_deposit: bool,
    tx_type: &str,
    idempotency_key: &str,
    description: &str,
) -> Result<Wallet, AppError> {
    if amount_vnd <= 0 {
        return Err(AppError::Validation("Số tiền cộng ví phải lớn hơn 0".to_string()));
    }

    let balance_before = wallet.balance_vnd;
    let balance_after = balance_before + amount_vnd;
    let deposit_delta = if is_deposit { amount_vnd } else { 0 };

    let updated = sqlx::query_as::<_, Wallet>(
        "UPDATE wallets
         SET balance_vnd = $1,
             total_deposited_vnd = total_deposited_vnd + $2,
             updated_at = NOW()
         WHERE id = $3
         RETURNING *",
    )
    .bind(balance_after)
    .bind(deposit_delta)
    .bind(wallet.id)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO wallet_transactions (
            wallet_id, tx_type, amount_vnd, balance_before, balance_after, idempotency_key, description
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(wallet.id)
    .bind(tx_type)
    .bind(amount_vnd)
    .bind(balance_before)
    .bind(balance_after)
    .bind(idempotency_key)
    .bind(description)
    .execute(&mut **tx)
    .await?;

    Ok(updated)
}

pub async fn debit_wallet(
    tx: &mut Transaction<'_, Postgres>,
    wallet: &Wallet,
    amount_vnd: i64,
    tx_type: &str,
    idempotency_key: &str,
    description: &str,
) -> Result<Wallet, AppError> {
    if amount_vnd <= 0 {
        return Err(AppError::Validation("Số tiền trừ ví phải lớn hơn 0".to_string()));
    }
    if wallet.balance_vnd < amount_vnd {
        return Err(AppError::InsufficientBalance {
            required: amount_vnd,
            available: wallet.balance_vnd,
        });
    }

    let balance_before = wallet.balance_vnd;
    let balance_after = balance_before - amount_vnd;

    let updated = sqlx::query_as::<_, Wallet>(
        "UPDATE wallets
         SET balance_vnd = $1, updated_at = NOW()
         WHERE id = $2
         RETURNING *",
    )
    .bind(balance_after)
    .bind(wallet.id)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO wallet_transactions (
            wallet_id, tx_type, amount_vnd, balance_before, balance_after, idempotency_key, description
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(wallet.id)
    .bind(tx_type)
    .bind(-amount_vnd)
    .bind(balance_before)
    .bind(balance_after)
    .bind(idempotency_key)
    .bind(description)
    .execute(&mut **tx)
    .await?;

    Ok(updated)
}

pub async fn list_recent_transactions(
    pool: &PgPool,
    wallet_id: Uuid,
    limit: i64,
) -> Result<Vec<WalletTransaction>, AppError> {
    let rows = sqlx::query_as::<_, WalletTransaction>(
        "SELECT * FROM wallet_transactions
         WHERE wallet_id = $1
         ORDER BY created_at DESC
         LIMIT $2",
    )
    .bind(wallet_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
