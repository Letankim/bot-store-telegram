use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    response::Html,
};
use serde::{Deserialize, Serialize};
use teloxide::{
    payloads::SetWebhookSetters,
    prelude::*,
    types::ParseMode,
};
use url::Url;
use uuid::Uuid;

use crate::{
    api::middleware::RequestContext,
    bot::{self, messages::escape_html},
    domain::{gemini_plan::GeminiPlan, settings::AppSettings, wallet::format_vnd},
    error::AppError,
    repo::{plan_repo, settings_repo, user_repo, wallet_repo},
    response::{ApiResponse, PaginationMeta},
    services::activation_service::AdminOrderView,
    state::SharedState,
};

const ADMIN_HTML: &str = include_str!("../../web/admin.html");

/// Serve Admin Configuration & Order Management Dashboard (`/admin`).
pub async fn serve_admin_page() -> Html<&'static str> {
    Html(ADMIN_HTML)
}

#[derive(Debug, Serialize)]
pub struct BotStatusRes {
    pub bot_connected: bool,
    pub bot_first_name: Option<String>,
    pub bot_username: Option<String>,
    pub bot_mode: String,
    pub payos_configured: bool,
    pub current_webhook_url: Option<String>,
    pub payos_webhook_url: String,
    pub guide_url: String,
}

/// Inspect live Telegram Bot connection (`getMe` + `getWebhookInfo`) and PayOS configuration.
pub async fn bot_status(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<BotStatusRes>, AppError> {
    let me = state.bot.get_me().await.ok();
    let webhook_info = state.bot.get_webhook_info().await.ok();

    let payos_configured = !state.config.payos_client_id.is_empty()
        && !state.config.payos_api_key.is_empty()
        && !state.config.payos_checksum_key.is_empty();

    let data = BotStatusRes {
        bot_connected: me.is_some(),
        bot_first_name: me.as_ref().map(|m| m.first_name.clone()),
        bot_username: me.and_then(|m| m.username.clone()),
        bot_mode: format!("{:?}", state.config.bot_mode),
        payos_configured,
        current_webhook_url: webhook_info
            .and_then(|w| w.url)
            .map(|u| u.to_string()),
        payos_webhook_url: format!("{}/api/v1/webhooks/payos", state.config.public_base_url),
        guide_url: state.config.guide_url(None),
    };

    Ok(ApiResponse::ok(
        data,
        "Kiểm tra trạng thái Bot thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct BotSetupReq {
    pub register_webhook: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct BotSetupRes {
    pub commands_registered: bool,
    pub webhook_registered: bool,
    pub webhook_url: Option<String>,
}

/// 1-click register Telegram slash commands and (optionally) Telegram Webhook.
pub async fn bot_setup(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<BotSetupReq>,
) -> Result<ApiResponse<BotSetupRes>, AppError> {
    bot::register_commands(&state.bot).await;

    let mut webhook_registered = false;
    let mut webhook_url_out = None;

    if payload.register_webhook.unwrap_or(false) {
        let webhook_url = format!("{}/api/v1/webhooks/telegram", state.config.public_base_url);
        let parsed = Url::parse(&webhook_url).map_err(|_| {
            AppError::Validation("PUBLIC_BASE_URL không phải URL hợp lệ.".to_string())
        })?;
        if parsed.scheme() != "https" {
            return Err(AppError::Validation(
                "Telegram Webhook bắt buộc dùng HTTPS (hãy cấu hình PUBLIC_BASE_URL với https:// của Render hoặc ngrok).".to_string(),
            ));
        }
        state
            .bot
            .set_webhook(parsed)
            .secret_token(state.config.telegram_webhook_secret.clone())
            .await
            .map_err(|e| AppError::Internal(format!("Đăng ký Webhook Telegram thất bại: {e}")))?;
        webhook_registered = true;
        webhook_url_out = Some(webhook_url);
    }

    Ok(ApiResponse::ok(
        BotSetupRes {
            commands_registered: true,
            webhook_registered,
            webhook_url: webhook_url_out,
        },
        if webhook_registered {
            "Đã đăng ký Menu lệnh và Webhook Telegram thành công!"
        } else {
            "Đã đăng ký Menu lệnh Bot Telegram thành công!"
        },
        ctx.request_id,
    ))
}

/// List all active Gemini Pro plans for Admin management.
pub async fn list_plans(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<Vec<GeminiPlan>>, AppError> {
    let plans = plan_repo::list_active_plans(&state.pool).await?;
    Ok(ApiResponse::ok(
        plans,
        "Lấy danh sách gói Gemini Pro thành công",
        ctx.request_id,
    ))
}

/// Get live system settings configurable from Admin Dashboard.
pub async fn get_settings(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<AppSettings>, AppError> {
    let settings = settings_repo::get_settings(&state.pool).await?;
    Ok(ApiResponse::ok(
        settings,
        "Lấy cấu hình hệ thống thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsReq {
    pub support_handle: String,
    pub min_deposit_vnd: i64,
    pub referral_reward_vnd: i64,
    pub referred_bonus_vnd: i64,
    pub referral_min_first_deposit_vnd: i64,
    pub quick_deposit_amounts: String,
}

/// Update live system settings from Admin Dashboard (applies immediately to Bot without restart).
pub async fn update_settings(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<UpdateSettingsReq>,
) -> Result<ApiResponse<AppSettings>, AppError> {
    let updated = settings_repo::update_settings(
        &state.pool,
        &payload.support_handle,
        payload.min_deposit_vnd,
        payload.referral_reward_vnd,
        payload.referred_bonus_vnd,
        payload.referral_min_first_deposit_vnd,
        &payload.quick_deposit_amounts,
    )
    .await?;

    Ok(ApiResponse::ok(
        updated,
        "Lưu cấu hình hệ thống thành công (áp dụng ngay lập tức)",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct ListOrdersQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// List activation orders with decrypted `email | pass | 2fa` for Admin processing.
pub async fn list_orders(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<ListOrdersQuery>,
) -> Result<ApiResponse<Vec<AdminOrderView>>, AppError> {
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(20).clamp(1, 100);

    let (orders, total) = state
        .activation_service
        .list_orders_for_admin(q.status.as_deref(), page, per_page)
        .await?;

    let total_pages = ((total as f64) / (per_page as f64)).ceil() as u32;

    Ok(ApiResponse::ok_paginated(
        orders,
        "Lấy danh sách đơn kích hoạt thành công",
        ctx.request_id,
        PaginationMeta {
            page,
            per_page,
            total_items: total.max(0) as u64,
            total_pages,
        },
    ))
}

#[derive(Debug, Deserialize)]
pub struct CompleteOrderReq {
    pub admin_note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OrderStatusRes {
    pub order_id: Uuid,
    pub order_code: String,
    pub status: String,
}

/// Mark an order COMPLETED and notify the customer via Telegram.
pub async fn complete_order(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(order_id): Path<Uuid>,
    Json(payload): Json<CompleteOrderReq>,
) -> Result<ApiResponse<OrderStatusRes>, AppError> {
    let outcome = state
        .activation_service
        .complete_order(order_id, payload.admin_note.as_deref())
        .await?;

    if let Some(tg_id) = outcome.user_telegram_id {
        let note_line = outcome
            .order
            .admin_note
            .as_deref()
            .map(|n| format!("\n📝 Ghi chú: {}", escape_html(n)))
            .unwrap_or_default();

        let msg = format!(
            "🎉 <b>Đơn Kích Hoạt Gemini Pro Đã Hoàn Tất!</b>\n\n\
             • Mã đơn: <code>{}</code>\n\
             • Gmail: <code>{}</code>{}\n\n\
             ✅ Bạn đã có thể đăng nhập Gemini Pro và sử dụng ngay. Cảm ơn bạn đã ủng hộ!",
            outcome.order.order_code,
            escape_html(&outcome.order.gmail_masked),
            note_line
        );
        let _ = state
            .bot
            .send_message(ChatId(tg_id), msg)
            .parse_mode(ParseMode::Html)
            .await;
    }

    Ok(ApiResponse::ok(
        OrderStatusRes {
            order_id: outcome.order.id,
            order_code: outcome.order.order_code,
            status: outcome.order.status,
        },
        "Đã hoàn tất đơn kích hoạt và thông báo khách hàng",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct RefundOrderReq {
    pub reason: String,
}

/// Refund 100% to user wallet for a failed/defective order and notify via Telegram.
pub async fn refund_order(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(order_id): Path<Uuid>,
    Json(payload): Json<RefundOrderReq>,
) -> Result<ApiResponse<OrderStatusRes>, AppError> {
    if payload.reason.trim().is_empty() {
        return Err(AppError::Validation("Vui lòng nhập lý do hoàn tiền.".to_string()));
    }

    let outcome = state
        .activation_service
        .refund_failed_order(order_id, payload.reason.trim())
        .await?;

    if let Some(tg_id) = outcome.user_telegram_id {
        let new_bal = outcome.refunded_balance_vnd.unwrap_or(0);
        let msg = format!(
            "↩️ <b>Hoàn Tiền 100% Đơn Hàng Lỗi!</b>\n\n\
             • Mã đơn: <code>{}</code>\n\
             • Số tiền hoàn lại ví: <b>+{} đ</b>\n\
             • Số dư ví hiện tại: <b>{} đ</b>\n\
             • Lý do: <i>{}</i>\n\n\
             📘 Vui lòng kiểm tra lại Trang Hướng Dẫn (/guide) trước khi đặt lại đơn mới nhé!",
            outcome.order.order_code,
            format_vnd(outcome.order.price_paid_vnd),
            format_vnd(new_bal),
            escape_html(payload.reason.trim()),
        );
        let _ = state
            .bot
            .send_message(ChatId(tg_id), msg)
            .parse_mode(ParseMode::Html)
            .await;
    }

    Ok(ApiResponse::ok(
        OrderStatusRes {
            order_id: outcome.order.id,
            order_code: outcome.order.order_code,
            status: outcome.order.status,
        },
        "Đã hoàn tiền 100% về ví khách hàng",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePlanReq {
    pub price_vnd: Option<i64>,
    pub available_stock: Option<i32>,
    pub is_active: Option<bool>,
}

/// Update Gemini Pro plan price or stock.
pub async fn update_plan(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(plan_id): Path<Uuid>,
    Json(payload): Json<UpdatePlanReq>,
) -> Result<ApiResponse<GeminiPlan>, AppError> {
    let updated = plan_repo::update_plan_stock_and_price(
        &state.pool,
        plan_id,
        payload.price_vnd,
        payload.available_stock,
        payload.is_active,
    )
    .await?;

    Ok(ApiResponse::ok(
        updated,
        "Cập nhật gói Gemini Pro thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct ManualCreditReq {
    pub amount_vnd: i64,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WalletBalanceRes {
    pub telegram_id: i64,
    pub balance_vnd: i64,
}

/// Admin manual wallet top-up (useful for testing or manual bank transfer support).
pub async fn manual_credit_wallet(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(telegram_id): Path<i64>,
    Json(payload): Json<ManualCreditReq>,
) -> Result<ApiResponse<WalletBalanceRes>, AppError> {
    let user = user_repo::find_by_telegram_id(&state.pool, telegram_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Không tìm thấy user với telegram_id này".to_string()))?;

    let mut tx = state.pool.begin().await?;
    let wallet = wallet_repo::lock_by_user_id(&mut tx, user.id).await?;
    let idem_key = format!("admin_adj_{}", Uuid::new_v4().simple());
    let desc = payload
        .description
        .unwrap_or_else(|| "Admin cộng tiền thủ công".to_string());

    let updated = wallet_repo::credit_wallet(
        &mut tx,
        &wallet,
        payload.amount_vnd,
        false,
        "ADMIN_ADJUST",
        &idem_key,
        &desc,
    )
    .await?;
    tx.commit().await?;

    Ok(ApiResponse::ok(
        WalletBalanceRes {
            telegram_id,
            balance_vnd: updated.balance_vnd,
        },
        "Cộng tiền ví thủ công thành công",
        ctx.request_id,
    ))
}
