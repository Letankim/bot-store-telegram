use chrono::Utc;
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    crypto::{sign_payos_create_payment, verify_payos_webhook_signature},
    domain::{
        payos::{
            PayOsCreatePaymentRequest, PayOsCreatePaymentResponse, PayOsDeposit,
            PayOsWebhookPayload,
        },
        wallet::format_vnd,
    },
    error::AppError,
    repo::{payos_repo, settings_repo, user_repo, wallet_repo},
    services::referral_service::{self, ReferralRewardResult},
};

#[derive(Clone)]
pub struct PayOsService {
    pool: PgPool,
    http: Client,
    config: Config,
}

#[derive(Debug, Clone)]
pub struct CreatedTopupLink {
    pub deposit: PayOsDeposit,
    pub qr_code: String,
    pub account_number: Option<String>,
    pub account_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PayOsWebhookOutcome {
    pub already_processed: bool,
    pub order_code: i64,
    pub user_telegram_id: Option<i64>,
    pub amount_vnd: i64,
    pub new_balance_vnd: i64,
    pub referral_reward: Option<ReferralRewardResult>,
}

#[derive(Debug, Clone)]
pub enum CheckPaymentStatus {
    PaidNow(PayOsWebhookOutcome),
    AlreadyPaid { order_code: i64, amount_vnd: i64 },
    StillPending { order_code: i64 },
    Cancelled { order_code: i64 },
}

#[derive(Debug, Deserialize)]
struct PayOsStatusApiResponse {
    code: String,
    data: Option<PayOsStatusData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PayOsStatusData {
    status: String,
    amount_paid: Option<i64>,
    amount: Option<i64>,
}

impl PayOsService {
    pub fn new(pool: PgPool, config: Config) -> Self {
        Self {
            pool,
            http: Client::new(),
            config,
        }
    }

    /// Create a VietQR payment link for wallet top-up using live min_deposit_vnd from settings.
    pub async fn create_topup_link(
        &self,
        user_id: Uuid,
        amount_vnd: i64,
    ) -> Result<CreatedTopupLink, AppError> {
        let settings = settings_repo::get_settings(&self.pool).await?;
        if amount_vnd < settings.min_deposit_vnd {
            return Err(AppError::Validation(format!(
                "Số tiền nạp tối thiểu mỗi lần là {} đ.",
                format_vnd(settings.min_deposit_vnd)
            )));
        }
        if self.config.payos_client_id.is_empty()
            || self.config.payos_api_key.is_empty()
            || self.config.payos_checksum_key.is_empty()
        {
            return Err(AppError::PaymentGateway(
                "Cổng thanh toán tự động chưa được cấu hình khóa kết nối.".to_string(),
            ));
        }

        // Generate a unique positive integer orderCode safe for JS & PayOS (<= 9_007_199_254_740_991)
        let ts_part = Utc::now().timestamp() % 1_000_000_000;
        let rand_part: i64 = rand::thread_rng().gen_range(100..999);
        let order_code = ts_part * 1000 + rand_part;

        // PayOS requires description <= 25 chars
        let description = format!("NAP {order_code}");
        let return_url = format!("{}/webapp/guide", self.config.public_base_url);
        let cancel_url = format!("{}/webapp/guide", self.config.public_base_url);

        let signature = sign_payos_create_payment(
            amount_vnd,
            &cancel_url,
            &description,
            order_code,
            &return_url,
            &self.config.payos_checksum_key,
        )?;

        let req_body = PayOsCreatePaymentRequest {
            order_code,
            amount: amount_vnd,
            description: description.clone(),
            cancel_url,
            return_url,
            signature,
        };

        let url = format!("{}/v2/payment-requests", self.config.payos_base_url);
        let resp = self
            .http
            .post(&url)
            .header("x-client-id", &self.config.payos_client_id)
            .header("x-api-key", &self.config.payos_api_key)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| AppError::PaymentGateway(format!("Không thể tạo mã chuyển khoản: {e}")))?;

        let status = resp.status();
        let parsed: PayOsCreatePaymentResponse = resp.json().await.map_err(|e| {
            AppError::PaymentGateway(format!("Phản hồi từ cổng thanh toán không hợp lệ: {e}"))
        })?;

        if !status.is_success() || parsed.code != "00" {
            return Err(AppError::PaymentGateway(format!(
                "Không thể tạo giao dịch ({}): {}",
                parsed.code, parsed.desc
            )));
        }

        let link_data = parsed.data.ok_or_else(|| {
            AppError::PaymentGateway("Cổng thanh toán trả về thiếu dữ liệu QR".to_string())
        })?;

        let deposit = payos_repo::create_deposit(
            &self.pool,
            order_code,
            user_id,
            amount_vnd,
            &description,
            &link_data.checkout_url,
            &link_data.qr_code,
            &link_data.payment_link_id,
        )
        .await?;

        Ok(CreatedTopupLink {
            deposit,
            qr_code: link_data.qr_code,
            account_number: link_data.account_number,
            account_name: link_data.account_name,
        })
    }

    /// Manually check payment status when user clicks "Tôi đã thanh toán" in Telegram.
    pub async fn check_and_confirm_payment(
        &self,
        order_code: i64,
    ) -> Result<CheckPaymentStatus, AppError> {
        // First query live status from PayOS API
        let url = format!(
            "{}/v2/payment-requests/{}",
            self.config.payos_base_url, order_code
        );
        let resp = self
            .http
            .get(&url)
            .header("x-client-id", &self.config.payos_client_id)
            .header("x-api-key", &self.config.payos_api_key)
            .send()
            .await
            .map_err(|e| AppError::PaymentGateway(format!("Không thể kiểm tra giao dịch: {e}")))?;

        let parsed: PayOsStatusApiResponse = resp
            .json()
            .await
            .map_err(|e| AppError::PaymentGateway(format!("Lỗi đọc trạng thái giao dịch: {e}")))?;

        if parsed.code != "00" {
            return Ok(CheckPaymentStatus::StillPending { order_code });
        }

        let Some(data) = parsed.data else {
            return Ok(CheckPaymentStatus::StillPending { order_code });
        };

        match data.status.to_uppercase().as_str() {
            "PAID" => {
                let paid_amount = data.amount_paid.or(data.amount).unwrap_or(0);
                let outcome = self
                    .credit_order_idempotent(order_code, paid_amount, None)
                    .await?;
                if outcome.already_processed {
                    Ok(CheckPaymentStatus::AlreadyPaid {
                        order_code,
                        amount_vnd: outcome.amount_vnd,
                    })
                } else {
                    Ok(CheckPaymentStatus::PaidNow(outcome))
                }
            }
            "CANCELLED" | "EXPIRED" => Ok(CheckPaymentStatus::Cancelled { order_code }),
            _ => Ok(CheckPaymentStatus::StillPending { order_code }),
        }
    }

    /// Cancel a pending deposit order when user clicks "Hủy thanh toán".
    pub async fn cancel_payment(&self, order_code: i64, user_id: Uuid) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let Some(deposit) = payos_repo::lock_by_order_code(&mut tx, order_code).await? else {
            return Ok(false);
        };
        if deposit.user_id != user_id || deposit.status != "PENDING" {
            return Ok(false);
        }

        // Best-effort cancel on PayOS gateway
        let url = format!(
            "{}/v2/payment-requests/{}/cancel",
            self.config.payos_base_url, order_code
        );
        let _ = self
            .http
            .post(&url)
            .header("x-client-id", &self.config.payos_client_id)
            .header("x-api-key", &self.config.payos_api_key)
            .json(&serde_json::json!({ "cancellationReason": "Khách hàng hủy giao dịch trên Bot" }))
            .send()
            .await;

        payos_repo::mark_cancelled(&mut tx, deposit.id).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Verify PayOS HMAC-SHA256 webhook signature and atomically credit wallet + referral reward.
    pub async fn handle_webhook(
        &self,
        payload: PayOsWebhookPayload,
    ) -> Result<PayOsWebhookOutcome, AppError> {
        let is_valid = verify_payos_webhook_signature(
            &payload.data,
            &payload.signature,
            &self.config.payos_checksum_key,
        )?;

        if !is_valid {
            return Err(AppError::Unauthorized(
                "Chữ ký HMAC-SHA256 từ PayOS Webhook không hợp lệ.".to_string(),
            ));
        }

        let order_code = payload
            .data
            .get("orderCode")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                AppError::Validation("Thiếu orderCode trong PayOS webhook".to_string())
            })?;

        // PayOS test webhook ping check (orderCode == 123)
        if order_code == 123 {
            return Ok(PayOsWebhookOutcome {
                already_processed: true,
                order_code,
                user_telegram_id: None,
                amount_vnd: 0,
                new_balance_vnd: 0,
                referral_reward: None,
            });
        }

        let data_code = payload
            .data
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if payload.code != "00" || data_code != "00" {
            return Ok(PayOsWebhookOutcome {
                already_processed: true,
                order_code,
                user_telegram_id: None,
                amount_vnd: 0,
                new_balance_vnd: 0,
                referral_reward: None,
            });
        }

        let paid_amount = payload
            .data
            .get("amount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let reference = payload
            .data
            .get("reference")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        self.credit_order_idempotent(order_code, paid_amount, reference.as_deref())
            .await
    }

    async fn credit_order_idempotent(
        &self,
        order_code: i64,
        paid_amount: i64,
        reference: Option<&str>,
    ) -> Result<PayOsWebhookOutcome, AppError> {
        let mut tx = self.pool.begin().await?;

        let deposit = payos_repo::lock_by_order_code(&mut tx, order_code)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("Không tìm thấy giao dịch #{order_code}"))
            })?;

        if deposit.status == "PAID" {
            tx.commit().await?;
            return Ok(PayOsWebhookOutcome {
                already_processed: true,
                order_code,
                user_telegram_id: None,
                amount_vnd: deposit.amount_vnd,
                new_balance_vnd: 0,
                referral_reward: None,
            });
        }

        let credit_amount = if paid_amount > 0 {
            paid_amount
        } else {
            deposit.amount_vnd
        };

        payos_repo::mark_paid(&mut tx, deposit.id, reference).await?;

        let wallet = wallet_repo::lock_by_user_id(&mut tx, deposit.user_id).await?;
        let idem_key = format!("payos_order_{order_code}");
        let desc = format!("Nạp tiền tự động VietQR (#{order_code})");
        let updated_wallet = wallet_repo::credit_wallet(
            &mut tx,
            &wallet,
            credit_amount,
            true,
            "PAYOS_TOPUP",
            &idem_key,
            &desc,
        )
        .await?;

        let settings = settings_repo::get_settings(&self.pool).await?;
        let referral_reward = referral_service::process_first_deposit_reward(
            &mut tx,
            deposit.user_id,
            credit_amount,
            settings.referral_min_first_deposit_vnd,
        )
        .await?;

        let final_balance = referral_reward
            .as_ref()
            .and_then(|r| r.referred_new_balance_vnd)
            .unwrap_or(updated_wallet.balance_vnd);

        tx.commit().await?;

        let user = user_repo::find_by_id(&self.pool, deposit.user_id).await?;

        Ok(PayOsWebhookOutcome {
            already_processed: false,
            order_code,
            user_telegram_id: user.map(|u| u.telegram_id),
            amount_vnd: credit_amount,
            new_balance_vnd: final_balance,
            referral_reward,
        })
    }
}
