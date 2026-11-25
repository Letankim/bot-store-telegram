use axum::{Extension, Json, extract::State, http::HeaderMap};
use serde::Serialize;
use teloxide::{prelude::*, types::{ParseMode, Update}};

use crate::{
    api::middleware::RequestContext,
    bot::{self, messages::escape_html},
    crypto::constant_time_eq_str,
    domain::{payos::PayOsWebhookPayload, wallet::format_vnd},
    error::AppError,
    response::ApiResponse,
    state::SharedState,
};

#[derive(Debug, Serialize)]
pub struct WebhookAck {
    pub received: bool,
}

/// Telegram Bot Webhook endpoint (`POST /api/v1/webhooks/telegram`).
pub async fn telegram_webhook(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Json(update): Json<Update>,
) -> Result<ApiResponse<WebhookAck>, AppError> {
    let secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_eq_str(secret, &state.config.telegram_webhook_secret) {
        return Err(AppError::Unauthorized(
            "X-Telegram-Bot-Api-Secret-Token không hợp lệ.".to_string(),
        ));
    }

    let bot_clone = state.bot.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        bot::dispatch_update(bot_clone, update, state_clone).await;
    });

    Ok(ApiResponse::ok(
        WebhookAck { received: true },
        "Đã tiếp nhận Telegram update",
        ctx.request_id,
    ))
}

/// Dedicated Admin Bot Webhook endpoint (`POST /api/v1/webhooks/telegram-admin`).
pub async fn telegram_admin_webhook(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Json(update): Json<Update>,
) -> Result<ApiResponse<WebhookAck>, AppError> {
    let secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_eq_str(secret, &state.config.telegram_webhook_secret) {
        return Err(AppError::Unauthorized(
            "X-Telegram-Bot-Api-Secret-Token không hợp lệ.".to_string(),
        ));
    }

    if let Some(admin_bot) = state.admin_bot.clone() {
        let state_clone = state.clone();
        tokio::spawn(async move {
            bot::admin_bot::dispatch_update(admin_bot, update, state_clone).await;
        });
    }

    Ok(ApiResponse::ok(
        WebhookAck { received: true },
        "Đã tiếp nhận Admin Bot Telegram update",
        ctx.request_id,
    ))
}

/// PayOS Payment Webhook endpoint (`POST /api/v1/webhooks/payos`).
pub async fn payos_webhook(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<PayOsWebhookPayload>,
) -> Result<ApiResponse<WebhookAck>, AppError> {
    let outcome = state.payos_service.handle_webhook(payload).await?;

    if !outcome.already_processed {
        if let Some(tg_id) = outcome.user_telegram_id.filter(|&id| id > 0) {
            let bonus_note = outcome
                .referral_reward
                .as_ref()
                .filter(|r| r.referred_bonus_vnd > 0)
                .map(|r| {
                    format!(
                        "\n🎁 Thưởng chào mừng nạp lần đầu (qua link mời): <b>+{} đ</b>",
                        format_vnd(r.referred_bonus_vnd)
                    )
                })
                .unwrap_or_default();

            let msg = format!(
                "✅ <b>Nạp tiền vào ví thành công!</b>\n\n\
                 • Mã giao dịch: <code>{}</code>\n\
                 • Số tiền đã nạp: <b>+{} đ</b>{}\n\
                 • Số dư ví hiện tại: <b>{} đ</b>\n\n\
                 🚀 Bấm <b>🛍️ Sản phẩm</b> (hoặc /shop) để mua hàng ngay bây giờ!",
                outcome.order_code,
                format_vnd(outcome.amount_vnd),
                bonus_note,
                format_vnd(outcome.new_balance_vnd),
            );
            let _ = state
                .bot
                .send_message(ChatId(tg_id), msg)
                .parse_mode(ParseMode::Html)
                .await;
        }

        if let Some(ref_reward) = outcome.referral_reward {
            if ref_reward.referrer_telegram_id > 0 {
                let msg = format!(
                    "🎉 <b>Bạn vừa nhận thưởng giới thiệu!</b>\n\n\
                     👤 <b>{}</b> vừa hoàn tất lần nạp tiền đầu tiên.\n\
                     💰 Đã cộng <b>+{} đ</b> vào ví của bạn!",
                    escape_html(&ref_reward.referred_user_name),
                    format_vnd(ref_reward.reward_vnd),
                );
                let _ = state
                    .bot
                    .send_message(ChatId(ref_reward.referrer_telegram_id), msg)
                    .parse_mode(ParseMode::Html)
                    .await;
            }
        }
    }

    Ok(ApiResponse::ok(
        WebhookAck { received: true },
        "Xử lý PayOS Webhook thành công",
        ctx.request_id,
    ))
}
