use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    crypto::CryptoService,
    domain::{
        activation_order::{ActivationOrder, NormalizedCredentials},
        gemini_plan::GeminiPlan,
        user::User,
    },
    error::AppError,
    repo::{activation_repo, plan_repo, user_repo, wallet_repo},
};

#[derive(Clone)]
pub struct ActivationService {
    pool: PgPool,
    crypto: CryptoService,
}

#[derive(Debug, Serialize)]
pub struct AdminOrderView {
    pub id: Uuid,
    pub order_code: String,
    pub user_id: Uuid,
    pub plan_id: Uuid,
    pub plan_name: String,
    pub plan_api_mode: String,
    pub plan_form_mode: String,
    pub price_paid_vnd: i64,
    pub gmail_masked: String,
    pub credentials_formatted: String,
    pub status: String,
    pub admin_note: Option<String>,
    pub external_job_id: Option<i64>,
    pub external_status: Option<String>,
    pub external_step: Option<String>,
    pub external_result: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug)]
pub struct SubmittedOrderOutcome {
    pub order: ActivationOrder,
    pub plan: GeminiPlan,
    pub user: User,
    pub remaining_balance_vnd: i64,
    pub formatted_credentials: String,
    pub normalized: NormalizedCredentials,
}

#[derive(Debug)]
pub struct OrderActionOutcome {
    pub order: ActivationOrder,
    pub user_telegram_id: Option<i64>,
    pub user_full_name: Option<String>,
    pub plan_name: Option<String>,
    pub refunded_balance_vnd: Option<i64>,
}

impl ActivationService {
    pub fn new(pool: PgPool, crypto: CryptoService) -> Self {
        Self { pool, crypto }
    }

    /// Submit a new product order from Telegram Mini App or direct Telegram buy.
    /// Runs balance verification, stock decrement, wallet debit, AES-256-GCM encryption in 1 ACID tx.
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_activation_order(
        &self,
        telegram_id: i64,
        plan_id: Uuid,
        email: &str,
        password: &str,
        two_fa_raw: &str,
        backup_codes: Option<&[String]>,
        custom_input: Option<&str>,
    ) -> Result<SubmittedOrderOutcome, AppError> {
        let user = user_repo::find_by_telegram_id(&self.pool, telegram_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(
                    "Tài khoản chưa khởi tạo. Vui lòng gõ /start trong Bot trước.".to_string(),
                )
            })?;

        self.submit_activation_order_for_user(
            user,
            plan_id,
            email,
            password,
            two_fa_raw,
            backup_codes,
            custom_input,
        )
        .await
    }

    /// Submit a new product order for an authenticated User (Web or Telegram).
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_activation_order_for_user(
        &self,
        user: User,
        plan_id: Uuid,
        email: &str,
        password: &str,
        two_fa_raw: &str,
        backup_codes: Option<&[String]>,
        custom_input: Option<&str>,
    ) -> Result<SubmittedOrderOutcome, AppError> {
        if user.is_banned {
            return Err(AppError::Forbidden(
                "Tài khoản của bạn đã bị khóa, không thể đặt đơn.".to_string(),
            ));
        }

        let current_plan = plan_repo::find_by_id(&self.pool, plan_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Không tìm thấy gói dịch vụ.".to_string()))?;

        if current_plan.is_maintenance {
            let note = if current_plan.maintenance_note.trim().is_empty() {
                "Vui lòng thử lại sau."
            } else {
                current_plan.maintenance_note.trim()
            };
            return Err(AppError::Maintenance(format!(
                "{} đang bảo trì. {}",
                current_plan.name, note
            )));
        }

        let normalized = NormalizedCredentials::build_for_form_mode(
            &current_plan.form_mode,
            email,
            password,
            two_fa_raw,
            backup_codes,
            custom_input,
        )?;
        let (ciphertext, nonce) = self.crypto.encrypt(&normalized.formatted_line)?;

        let mut tx = self.pool.begin().await?;

        let plan = plan_repo::decrement_stock_and_increment_sold(&mut tx, plan_id).await?;
        let wallet = wallet_repo::lock_by_user_id(&mut tx, user.id).await?;

        let order_code = format!(
            "GP{}",
            &Uuid::new_v4().simple().to_string()[..8].to_uppercase()
        );
        let idem_key = format!("act_order_{order_code}");
        let desc = format!("Kích hoạt {} ({})", plan.name, normalized.gmail_masked);

        let updated_wallet = wallet_repo::debit_wallet(
            &mut tx,
            &wallet,
            plan.price_vnd,
            "GEMINI_ACTIVATION",
            &idem_key,
            &desc,
        )
        .await?;

        let order = activation_repo::create_order(
            &mut tx,
            &order_code,
            user.id,
            plan.id,
            plan.price_vnd,
            &normalized.gmail_masked,
            &ciphertext,
            &nonce,
        )
        .await?;

        tx.commit().await?;

        let formatted_credentials = normalized.formatted_line.clone();
        Ok(SubmittedOrderOutcome {
            order,
            plan,
            user,
            remaining_balance_vnd: updated_wallet.balance_vnd,
            formatted_credentials,
            normalized,
        })
    }

    /// Decrypt an existing order's credentials along with its plan `api_mode` & user telegram ID.
    pub async fn load_decrypted_order(
        &self,
        order_id: Uuid,
    ) -> Result<(ActivationOrder, NormalizedCredentials, Option<i64>, String), AppError> {
        let order = activation_repo::find_by_id(&self.pool, order_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Không tìm thấy đơn kích hoạt".to_string()))?;

        let decrypted = self
            .crypto
            .decrypt(&order.credentials_encrypted, &order.nonce)?;
        let parsed = NormalizedCredentials::parse_formatted(&decrypted)?;
        let tg_id = user_repo::find_by_id(&self.pool, order.user_id)
            .await?
            .map(|u| u.telegram_id);
        let plan_api_mode = plan_repo::find_by_id(&self.pool, order.plan_id)
            .await?
            .map(|p| p.api_mode)
            .unwrap_or_else(|| "autopay".to_string());

        Ok((order, parsed, tg_id, plan_api_mode))
    }

    /// List orders for Admin API with decrypted `email | pass | 2fa` string and plan `api_mode`.
    pub async fn list_orders_for_admin(
        &self,
        status: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<AdminOrderView>, i64), AppError> {
        let per_page_clamped = per_page.clamp(1, 100);
        let page_clamped = page.max(1);
        let offset = i64::from((page_clamped - 1) * per_page_clamped);

        let (orders, total) =
            activation_repo::list_for_admin(&self.pool, status, i64::from(per_page_clamped), offset)
                .await?;

        let all_plans = plan_repo::list_all_plans(&self.pool).await.unwrap_or_default();

        let mut views = Vec::with_capacity(orders.len());
        for order in orders {
            let decrypted = self
                .crypto
                .decrypt(&order.credentials_encrypted, &order.nonce)?;
            let plan = all_plans.iter().find(|p| p.id == order.plan_id);
            views.push(AdminOrderView {
                id: order.id,
                order_code: order.order_code,
                user_id: order.user_id,
                plan_id: order.plan_id,
                plan_name: plan
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Gemini Pro".to_string()),
                plan_api_mode: plan
                    .map(|p| p.api_mode.clone())
                    .unwrap_or_else(|| "autopay".to_string()),
                plan_form_mode: plan
                    .map(|p| p.form_mode.clone())
                    .unwrap_or_else(|| "mail_pass_2fa".to_string()),
                price_paid_vnd: order.price_paid_vnd,
                gmail_masked: order.gmail_masked,
                credentials_formatted: decrypted,
                status: order.status,
                admin_note: order.admin_note,
                external_job_id: order.external_job_id,
                external_status: order.external_status,
                external_step: order.external_step,
                external_result: order.external_result,
                created_at: order.created_at,
            });
        }

        Ok((views, total))
    }

    /// Admin marks an activation order as COMPLETED.
    pub async fn complete_order(
        &self,
        order_id: Uuid,
        admin_note: Option<&str>,
    ) -> Result<OrderActionOutcome, AppError> {
        let mut tx = self.pool.begin().await?;
        let order = activation_repo::lock_by_id(&mut tx, order_id).await?;
        if order.status != "PENDING" && order.status != "PROCESSING" {
            return Err(AppError::Conflict(format!(
                "Đơn {} đang ở trạng thái {}, không thể hoàn tất.",
                order.order_code, order.status
            )));
        }

        let updated = activation_repo::mark_completed(&mut tx, order_id, admin_note).await?;
        tx.commit().await?;

        let user = user_repo::find_by_id(&self.pool, updated.user_id).await?;
        let plan_name = plan_repo::find_by_id(&self.pool, updated.plan_id)
            .await?
            .map(|p| p.name);
        Ok(OrderActionOutcome {
            order: updated,
            user_telegram_id: user.as_ref().map(|u| u.telegram_id),
            user_full_name: user.map(|u| u.full_name),
            plan_name,
            refunded_balance_vnd: None,
        })
    }

    /// Admin refunds 100% to user wallet for a defective/failed activation order.
    pub async fn refund_failed_order(
        &self,
        order_id: Uuid,
        reason: &str,
    ) -> Result<OrderActionOutcome, AppError> {
        let mut tx = self.pool.begin().await?;
        let order = activation_repo::lock_by_id(&mut tx, order_id).await?;
        if order.status != "PENDING" && order.status != "PROCESSING" {
            return Err(AppError::Conflict(format!(
                "Đơn {} đang ở trạng thái {}, không thể hoàn tiền.",
                order.order_code, order.status
            )));
        }

        let wallet = wallet_repo::lock_by_user_id(&mut tx, order.user_id).await?;
        let idem_key = format!("refund_order_{}", order.id);
        let desc = format!("Hoàn tiền đơn lỗi {} ({reason})", order.order_code);

        let updated_wallet = wallet_repo::credit_wallet(
            &mut tx,
            &wallet,
            order.price_paid_vnd,
            false,
            "ORDER_REFUND",
            &idem_key,
            &desc,
        )
        .await?;

        let updated = activation_repo::mark_refunded(&mut tx, order_id, reason).await?;
        plan_repo::restore_stock_on_refund(&mut tx, order.plan_id).await?;
        tx.commit().await?;

        let user = user_repo::find_by_id(&self.pool, updated.user_id).await?;
        let plan_name = plan_repo::find_by_id(&self.pool, updated.plan_id)
            .await?
            .map(|p| p.name);
        Ok(OrderActionOutcome {
            order: updated,
            user_telegram_id: user.as_ref().map(|u| u.telegram_id),
            user_full_name: user.map(|u| u.full_name),
            plan_name,
            refunded_balance_vnd: Some(updated_wallet.balance_vnd),
        })
    }
}
