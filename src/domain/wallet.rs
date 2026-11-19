use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Wallet {
    pub id: Uuid,
    pub user_id: Uuid,
    pub balance_vnd: i64,
    pub total_deposited_vnd: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct WalletTransaction {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub tx_type: String,
    pub amount_vnd: i64,
    pub balance_before: i64,
    pub balance_after: i64,
    pub idempotency_key: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

/// Format VND integer cleanly, e.g. `40000` -> `"40.000"`.
pub fn format_vnd(amount: i64) -> String {
    let neg = amount < 0;
    let digits = amount.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    let formatted: String = out.chars().rev().collect();
    if neg {
        format!("-{formatted}")
    } else {
        formatted
    }
}
