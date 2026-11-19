use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{error::AppError, repo::wallet_repo};

#[derive(Debug, Clone)]
pub struct ReferralRewardResult {
    pub referrer_telegram_id: i64,
    pub reward_vnd: i64,
    pub referred_bonus_vnd: i64,
    pub referred_user_name: String,
    pub referred_new_balance_vnd: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InvitedFriendItem {
    pub full_name: String,
    pub status: String,
    pub reward_vnd: i64,
}

#[derive(Debug, Clone)]
pub struct ReferralSummary {
    pub total_invited: i64,
    pub rewarded_count: i64,
    pub pending_count: i64,
    pub total_earned_vnd: i64,
    pub recent_friends: Vec<InvitedFriendItem>,
}

#[derive(sqlx::FromRow)]
struct PendingReferralRow {
    id: Uuid,
    referrer_id: Uuid,
    reward_vnd: i64,
    referred_bonus_vnd: i64,
    referrer_telegram_id: i64,
    referred_user_name: String,
}

#[derive(sqlx::FromRow)]
struct ReferralStatsRow {
    total_invited: Option<i64>,
    rewarded_count: Option<i64>,
    pending_count: Option<i64>,
    total_earned_vnd: Option<i64>,
}

/// Fetch personal referral statistics & recent invited friends for `/ref` menu.
pub async fn get_referral_summary(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<ReferralSummary, AppError> {
    let stats = sqlx::query_as::<_, ReferralStatsRow>(
        "SELECT
            COUNT(*)::BIGINT AS total_invited,
            COUNT(*) FILTER (WHERE status = 'REWARDED')::BIGINT AS rewarded_count,
            COUNT(*) FILTER (WHERE status = 'PENDING_FIRST_DEPOSIT')::BIGINT AS pending_count,
            COALESCE(SUM(reward_vnd) FILTER (WHERE status = 'REWARDED'), 0)::BIGINT AS total_earned_vnd
         FROM referrals
         WHERE referrer_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    let recent_friends = sqlx::query_as::<_, InvitedFriendItem>(
        "SELECT u.full_name, r.status, r.reward_vnd
         FROM referrals r
         JOIN users u ON u.id = r.referred_user_id
         WHERE r.referrer_id = $1
         ORDER BY r.created_at DESC
         LIMIT 15",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(ReferralSummary {
        total_invited: stats.total_invited.unwrap_or(0),
        rewarded_count: stats.rewarded_count.unwrap_or(0),
        pending_count: stats.pending_count.unwrap_or(0),
        total_earned_vnd: stats.total_earned_vnd.unwrap_or(0),
        recent_friends,
    })
}

/// Award referral bonus to BOTH referrer and invited friend inside the same DB transaction
/// when the invited friend completes their first qualifying deposit.
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
        "SELECT r.id, r.referrer_id, r.reward_vnd, r.referred_bonus_vnd,
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

    // 1. Credit referrer wallet
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

    // 2. Credit invited friend bonus wallet (if configured > 0)
    let mut referred_new_balance = None;
    if row.referred_bonus_vnd > 0 {
        let friend_wallet = wallet_repo::lock_by_user_id(tx, referred_user_id).await?;
        let idem_key = format!("ref_friend_bonus_{}", row.id);
        let desc = "Thưởng chào mừng người được mời nạp lần đầu".to_string();

        let updated_friend = wallet_repo::credit_wallet(
            tx,
            &friend_wallet,
            row.referred_bonus_vnd,
            false,
            "REFERRAL_REWARD",
            &idem_key,
            &desc,
        )
        .await?;
        referred_new_balance = Some(updated_friend.balance_vnd);
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
        referred_bonus_vnd: row.referred_bonus_vnd,
        referred_user_name: row.referred_user_name,
        referred_new_balance_vnd: referred_new_balance,
    }))
}
