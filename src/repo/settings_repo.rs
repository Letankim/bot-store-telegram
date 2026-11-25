use sqlx::PgPool;

use crate::{domain::settings::AppSettings, error::AppError};

pub async fn get_settings(pool: &PgPool) -> Result<AppSettings, AppError> {
    let row = sqlx::query_as::<_, AppSettings>("SELECT * FROM app_settings WHERE id = 1")
        .fetch_one(pool)
        .await?;
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_settings(
    pool: &PgPool,
    support_handle: &str,
    min_deposit_vnd: i64,
    referral_reward_vnd: i64,
    referred_bonus_vnd: i64,
    referral_min_first_deposit_vnd: i64,
    quick_deposit_amounts: &str,
    smm_api_base_url: Option<&str>,
    smm_api_key: Option<&str>,
    smm_auto_push: Option<bool>,
    smm_default_mode: Option<&str>,
    smm_auto_refund_on_fail: Option<bool>,
) -> Result<AppSettings, AppError> {
    if min_deposit_vnd < 1_000 {
        return Err(AppError::Validation(
            "Số tiền nạp tối thiểu phải từ 1.000 đ trở lên.".to_string(),
        ));
    }
    if referral_reward_vnd < 0 || referred_bonus_vnd < 0 || referral_min_first_deposit_vnd < 0 {
        return Err(AppError::Validation(
            "Giá trị thưởng và điều kiện nạp lần đầu không được âm.".to_string(),
        ));
    }

    let handle_clean = support_handle.trim();
    if handle_clean.is_empty() {
        return Err(AppError::Validation(
            "Tài khoản hỗ trợ không được để trống.".to_string(),
        ));
    }

    if let Some(mode) = smm_default_mode {
        if !matches!(mode, "autopay" | "payadd" | "extract") {
            return Err(AppError::Validation(
                "Chế độ API chỉ hỗ trợ: autopay, payadd, hoặc extract.".to_string(),
            ));
        }
    }

    let updated = sqlx::query_as::<_, AppSettings>(
        "UPDATE app_settings
         SET support_handle = $1,
             min_deposit_vnd = $2,
             referral_reward_vnd = $3,
             referred_bonus_vnd = $4,
             referral_min_first_deposit_vnd = $5,
             quick_deposit_amounts = $6,
             smm_api_base_url = COALESCE($7, smm_api_base_url),
             smm_api_key = COALESCE($8, smm_api_key),
             smm_auto_push = COALESCE($9, smm_auto_push),
             smm_default_mode = COALESCE($10, smm_default_mode),
             smm_auto_refund_on_fail = COALESCE($11, smm_auto_refund_on_fail),
             updated_at = NOW()
         WHERE id = 1
         RETURNING *",
    )
    .bind(handle_clean)
    .bind(min_deposit_vnd)
    .bind(referral_reward_vnd)
    .bind(referred_bonus_vnd)
    .bind(referral_min_first_deposit_vnd)
    .bind(quick_deposit_amounts.trim())
    .bind(smm_api_base_url.map(|s| s.trim().trim_end_matches('/')))
    .bind(smm_api_key.map(str::trim))
    .bind(smm_auto_push)
    .bind(smm_default_mode)
    .bind(smm_auto_refund_on_fail)
    .fetch_one(pool)
    .await?;

    Ok(updated)
}

/// Update Partner Shop API & Telethon Auto-Sync configuration
/// and immediately recalculate sell prices for all products using the default markup.
#[allow(clippy::too_many_arguments)]
pub async fn update_partner_shop_settings(
    pool: &PgPool,
    base_url: &str,
    api_key: Option<&str>,
    enabled: bool,
    markup_vnd: i64,
    sync_interval_secs: i32,
    notify_changes: bool,
    auto_sync_worker_url: Option<&str>,
    telethon_auto_sync: Option<bool>,
) -> Result<AppSettings, AppError> {
    if markup_vnd < 0 {
        return Err(AppError::Validation(
            "Giá chênh lệch (lợi nhuận) không được nhỏ hơn 0.".to_string(),
        ));
    }
    let clamped_interval = sync_interval_secs.clamp(30, 86_400);
    let clean_url = base_url.trim().trim_end_matches('/');
    if clean_url.is_empty() {
        return Err(AppError::Validation(
            "Base URL Shop đối tác không được để trống.".to_string(),
        ));
    }
    let clean_worker_url = auto_sync_worker_url.map(|u| u.trim().trim_end_matches('/'));
    if let Some(wu) = clean_worker_url {
        if !wu.is_empty() && !wu.starts_with("http://") && !wu.starts_with("https://") {
            return Err(AppError::Validation(
                "URL Worker Auto-Sync phải bắt đầu bằng https:// hoặc http://".to_string(),
            ));
        }
    }

    let updated = sqlx::query_as::<_, AppSettings>(
        "UPDATE app_settings
         SET partner_shop_base_url = $1,
             partner_shop_api_key = COALESCE($2, partner_shop_api_key),
             partner_shop_enabled = $3,
             partner_shop_markup_vnd = $4,
             partner_shop_sync_interval_secs = $5,
             partner_shop_notify_changes = $6,
             auto_sync_worker_url = COALESCE($7, auto_sync_worker_url),
             telethon_auto_sync = COALESCE($8, telethon_auto_sync),
             updated_at = NOW()
         WHERE id = 1
         RETURNING *",
    )
    .bind(clean_url)
    .bind(api_key.map(str::trim).filter(|s| !s.is_empty()))
    .bind(enabled)
    .bind(markup_vnd)
    .bind(clamped_interval)
    .bind(notify_changes)
    .bind(clean_worker_url)
    .bind(telethon_auto_sync)
    .fetch_one(pool)
    .await?;

    // Recalculate sell_price_vnd for all synced products that inherit the global markup
    sqlx::query(
        "UPDATE partner_products
         SET sell_price_vnd = partner_price_vnd + $1,
             updated_at = NOW()
         WHERE custom_markup_vnd IS NULL",
    )
    .bind(markup_vnd)
    .execute(pool)
    .await?;

    Ok(updated)
}
