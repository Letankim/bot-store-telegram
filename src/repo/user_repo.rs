use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{domain::user::User, error::AppError};

pub async fn find_by_telegram_id(pool: &PgPool, telegram_id: i64) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE telegram_id = $1")
        .bind(telegram_id)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

pub async fn find_by_id(pool: &PgPool, user_id: Uuid) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

pub async fn find_by_ref_code(pool: &PgPool, ref_code: &str) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE ref_code = $1")
        .bind(ref_code)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// Upsert user and ensure a wallet exists. Returns `(User, is_newly_created)`.
pub async fn ensure_user_with_wallet(
    tx: &mut Transaction<'_, Postgres>,
    telegram_id: i64,
    username: Option<&str>,
    full_name: &str,
    referred_by: Option<Uuid>,
    referral_reward_vnd: i64,
    referred_bonus_vnd: i64,
) -> Result<(User, bool), AppError> {
    if let Some(existing) = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE telegram_id = $1 FOR UPDATE",
    )
    .bind(telegram_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        let updated = sqlx::query_as::<_, User>(
            "UPDATE users SET username = $1, full_name = $2, updated_at = NOW() WHERE id = $3 RETURNING *",
        )
        .bind(username)
        .bind(full_name)
        .bind(existing.id)
        .fetch_one(&mut **tx)
        .await?;

        return Ok((updated, false));
    }

    let ref_code = format!("REF{}", &Uuid::new_v4().simple().to_string()[..8].to_uppercase());
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (telegram_id, username, full_name, ref_code, referred_by)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING *",
    )
    .bind(telegram_id)
    .bind(username)
    .bind(full_name)
    .bind(&ref_code)
    .bind(referred_by)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query("INSERT INTO wallets (user_id, balance_vnd, total_deposited_vnd) VALUES ($1, 0, 0)")
        .bind(user.id)
        .execute(&mut **tx)
        .await?;

    if let Some(referrer_id) = referred_by {
        sqlx::query(
            "INSERT INTO referrals (referrer_id, referred_user_id, reward_vnd, referred_bonus_vnd, status)
             VALUES ($1, $2, $3, $4, 'PENDING_FIRST_DEPOSIT')
             ON CONFLICT (referred_user_id) DO NOTHING",
        )
        .bind(referrer_id)
        .bind(user.id)
        .bind(referral_reward_vnd)
        .bind(referred_bonus_vnd)
        .execute(&mut **tx)
        .await?;
    }

    Ok((user, true))
}
