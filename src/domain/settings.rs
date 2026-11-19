use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AppSettings {
    pub id: i16,
    pub support_handle: String,
    pub min_deposit_vnd: i64,
    pub referral_reward_vnd: i64,
    pub referral_min_first_deposit_vnd: i64,
    pub quick_deposit_amounts: String,
    pub updated_at: DateTime<Utc>,
}

impl AppSettings {
    /// Parse comma-separated quick deposit amounts (e.g. `"40000,80000,150000,300000"`).
    pub fn parsed_quick_amounts(&self) -> Vec<i64> {
        let parsed: Vec<i64> = self
            .quick_deposit_amounts
            .split(',')
            .filter_map(|s| s.trim().parse::<i64>().ok())
            .filter(|&v| v >= self.min_deposit_vnd)
            .collect();

        if parsed.is_empty() {
            vec![
                self.min_deposit_vnd.max(40_000),
                self.min_deposit_vnd.max(80_000),
                self.min_deposit_vnd.max(150_000),
                self.min_deposit_vnd.max(300_000),
            ]
        } else {
            parsed
        }
    }
}
