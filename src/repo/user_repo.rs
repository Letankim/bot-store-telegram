use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{domain::user::User, error::AppError};

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AdminUserWithWallet {
    pub id: Uuid,
    pub telegram_id: i64,
    pub username: Option<String>,
    pub full_name: String,
    pub ref_code: String,
    pub role: String,
    pub is_banned: bool,
    pub created_at: DateTime<Utc>,
    pub wallet_id: Uuid,
    pub balance_vnd: i64,
    pub total_deposited_vnd: i64,
    pub orders_count: i64,
    pub referrals_count: i64,
}

pub async fn list_users_with_wallets(
    pool: &PgPool,
    search: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminUserWithWallet>, i64), AppError> {
    let q = search.map(str::trim).filter(|s| !s.is_empty());

    let (rows, total) = if let Some(term) = q {
        let like_pattern = format!("%{term}%");
        let rows = sqlx::query_as::<_, AdminUserWithWallet>(
            "SELECT
                u.id, u.telegram_id, u.username, u.full_name, u.ref_code, u.role, u.is_banned, u.created_at,
                w.id AS wallet_id, w.balance_vnd, w.total_deposited_vnd,
                (SELECT COUNT(*) FROM activation_orders o WHERE o.user_id = u.id)::BIGINT AS orders_count,
                (SELECT COUNT(*) FROM referrals r WHERE r.referrer_id = u.id)::BIGINT AS referrals_count
             FROM users u
             JOIN wallets w ON w.user_id = u.id
             WHERE u.full_name ILIKE $1
                OR COALESCE(u.username, '') ILIKE $1
                OR COALESCE(u.email, '') ILIKE $1
                OR CAST(u.telegram_id AS TEXT) ILIKE $1
             ORDER BY u.created_at DESC
             LIMIT $2 OFFSET $3",
        )
        .bind(&like_pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM users u
             WHERE u.full_name ILIKE $1
                OR COALESCE(u.username, '') ILIKE $1
                OR COALESCE(u.email, '') ILIKE $1
                OR CAST(u.telegram_id AS TEXT) ILIKE $1",
        )
        .bind(&like_pattern)
        .fetch_one(pool)
        .await?;

        (rows, total.0)
    } else {
        let rows = sqlx::query_as::<_, AdminUserWithWallet>(
            "SELECT
                u.id, u.telegram_id, u.username, u.full_name, u.ref_code, u.role, u.is_banned, u.created_at,
                w.id AS wallet_id, w.balance_vnd, w.total_deposited_vnd,
                (SELECT COUNT(*) FROM activation_orders o WHERE o.user_id = u.id)::BIGINT AS orders_count,
                (SELECT COUNT(*) FROM referrals r WHERE r.referrer_id = u.id)::BIGINT AS referrals_count
             FROM users u
             JOIN wallets w ON w.user_id = u.id
             ORDER BY u.created_at DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(pool)
            .await?;

        (rows, total.0)
    };

    Ok((rows, total))
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AdminUsersSummary {
    pub total_users: i64,
    pub total_deposited_vnd: i64,
    pub total_balance_vnd: i64,
    pub banned_users: i64,
}

pub async fn get_users_summary(pool: &PgPool) -> Result<AdminUsersSummary, AppError> {
    let row = sqlx::query_as::<_, AdminUsersSummary>(
        "SELECT
            COUNT(u.id)::BIGINT AS total_users,
            COALESCE(SUM(w.total_deposited_vnd), 0)::BIGINT AS total_deposited_vnd,
            COALESCE(SUM(w.balance_vnd), 0)::BIGINT AS total_balance_vnd,
            COUNT(u.id) FILTER (WHERE u.is_banned = TRUE)::BIGINT AS banned_users
         FROM users u
         JOIN wallets w ON w.user_id = u.id",
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_all_active_telegram_ids(pool: &PgPool) -> Result<Vec<i64>, AppError> {
    let ids: Vec<(i64,)> =
        sqlx::query_as("SELECT telegram_id FROM users WHERE is_banned = FALSE AND telegram_id > 0")
            .fetch_all(pool)
            .await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
}

pub async fn find_by_email(pool: &PgPool, email_lower: &str) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE LOWER(email) = $1")
        .bind(email_lower)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// Create a new Web-only user (without Telegram) + Wallet + optional Referral in 1 ACID tx.
/// Uses a unique negative `telegram_id` so Telegram broadcasts & queries stay cleanly separated.
pub async fn create_web_user_with_wallet(
    tx: &mut Transaction<'_, Postgres>,
    email_lower: &str,
    password_hash: &str,
    full_name: &str,
    referred_by: Option<Uuid>,
    referral_reward_vnd: i64,
    referred_bonus_vnd: i64,
) -> Result<User, AppError> {
    let web_tg_id: i64 = -(1_000_000_000_000_i64
        + (rand::random::<u64>() % 7_999_999_999_999) as i64);
    let ref_code = format!("W{}", &Uuid::new_v4().simple().to_string()[..7].to_uppercase());

    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (telegram_id, email, password_hash, username, full_name, ref_code, referred_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING *",
    )
    .bind(web_tg_id)
    .bind(email_lower)
    .bind(password_hash)
    .bind(Some(email_lower))
    .bind(full_name.trim())
    .bind(&ref_code)
    .bind(referred_by)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db_err) = &e {
            if db_err.is_unique_violation() {
                return AppError::Conflict("Email này đã được đăng ký tài khoản.".to_string());
            }
        }
        AppError::Database(e)
    })?;

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

    Ok(user)
}

pub async fn create_web_session(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO web_sessions (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_user_by_session_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>(
        "SELECT u.* FROM users u
         JOIN web_sessions s ON s.user_id = u.id
         WHERE s.token_hash = $1 AND s.expires_at > NOW()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(user)
}

pub async fn delete_web_session(pool: &PgPool, token_hash: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM web_sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_all_user_sessions(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM web_sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_user_banned(
    pool: &PgPool,
    user_id: Uuid,
    is_banned: bool,
) -> Result<User, AppError> {
    let updated = sqlx::query_as::<_, User>(
        "UPDATE users SET is_banned = $1, updated_at = NOW() WHERE id = $2 RETURNING *",
    )
    .bind(is_banned)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Không tìm thấy người dùng".to_string()))?;
    Ok(updated)
}

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
