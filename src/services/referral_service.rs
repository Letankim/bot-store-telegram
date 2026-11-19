use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{error::AppError, repo::wallet_repo};

#[derive(Debug, Clone)]
pub struct ReferralRewardResult {
    pub referrer_telegram_id: i64,
    pub reward_vnd: i64,
    pub referred_user_name: String,
}

#[derive(sqlx::FromRow)]
struct PendingReferralRow {
    id: Uuid,
    referrer_id: Uuid,
    reward_vnd: i64,
    referrer_telegram_id: i64,
    referred_user_name: String,
}

/// Award referral bonus inside the same DB transaction when a referred user completes their first deposit
/// meeting the configured minimum first deposit threshold.
pub async fn process_first_deposit_reward(
    tx: &mut Transaction<'_, Postgres>,
    referred_user_id: Uuid,
    deposited_amount_vnd: i64,
    min_first_deposit_vnd: i64,
) -> Result<Option<ReferralRewardResult>, AppError> {
    if deposited_amount_vnd < min_first_deposit_vnd {
        return Ok(None);
    }

    let pending = sqlx::query_as::<_, PendingReferralRow>(
        "SELECT r.id, r.referrer_id, r.reward_vnd,
                ref_u.telegram_id AS referrer_telegram_id,
                u.full_name AS referred_user_name
         FROM referrals r
         JOIN users ref_u ON ref_u.id = r.referrer_id
         JOIN users u ON u.id = r.referred_user_id
         WHERE r.referred_user_id = $1 AND r.status = 'PENDING_FIRST_DEPOSIT'
         FOR UPDATE OF r",
    )
    .bind(referred_user_id)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(row) = pending else {
        return Ok(None);
    };

    if row.reward_vnd > 0 {
        let referrer_wallet = wallet_repo::lock_by_user_id(tx, row.referrer_id).await?;
        let idem_key = format!("ref_reward_{}", row.id);
        let desc = format!("Thưởng giới thiệu lần nạp đầu từ {}", row.referred_user_name);

        wallet_repo::credit_wallet(
            tx,
            &referrer_wallet,
            row.reward_vnd,
            false,
            "REFERRAL_REWARD",
            &idem_key,
            &desc,
        )
        .await?;
    }

    sqlx::query(
        "UPDATE referrals SET status = 'REWARDED', rewarded_at = NOW() WHERE id = $1",
    )
    .bind(row.id)
    .execute(&mut **tx)
    .await?;

    Ok(Some(ReferralRewardResult {
        referrer_telegram_id: row.referrer_telegram_id,
        reward_vnd: row.reward_vnd,
        referred_user_name: row.referred_user_name,
    }))
}
