use axum::{
    Extension, Json,
    extract::{Query, State},
    http::HeaderMap,
    response::Html,
};
use serde::{Deserialize, Serialize};
use teloxide::{prelude::*, types::ParseMode};
use uuid::Uuid;

use crate::{
    api::middleware::RequestContext,
    bot::messages::escape_html,
    crypto::verify_telegram_webapp_init_data,
    domain::{gemini_plan::GeminiPlan, wallet::format_vnd},
    error::AppError,
    repo::{plan_repo, user_repo, wallet_repo},
    response::ApiResponse,
    state::SharedState,
};

const ACTIVATE_HTML: &str = include_str!("../../web/activate.html");
const GUIDE_HTML: &str = include_str!("../../web/guide.html");

/// Serve Telegram Mini App Form (`/webapp/activate`).
pub async fn serve_activate_page() -> Html<&'static str> {
    Html(ACTIVATE_HTML)
}

/// Serve Interactive 4-Step Guide Page (`/webapp/guide`).
pub async fn serve_guide_page() -> Html<&'static str> {
    Html(GUIDE_HTML)
}

#[derive(Debug, Deserialize)]
pub struct WebAppMeQuery {
    pub plan_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WebAppMeData {
    pub telegram_id: i64,
    pub full_name: String,
    pub balance_vnd: i64,
    pub plan: GeminiPlan,
}

/// Authenticate Telegram Mini App `initData` and return user wallet balance + selected Gemini Pro plan.
pub async fn webapp_me(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Query(query): Query<WebAppMeQuery>,
) -> Result<ApiResponse<WebAppMeData>, AppError> {
    let init_data = headers
        .get("x-telegram-init-data")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let tg_user =
        verify_telegram_webapp_init_data(init_data, &state.config.telegram_bot_token, 3600)?;

    let user = user_repo::find_by_telegram_id(&state.pool, tg_user.id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound("Vui lòng gõ /start trong Bot trước khi mở Form.".to_string())
        })?;

    let wallet = wallet_repo::get_by_user_id(&state.pool, user.id).await?;

    let plan = if let Some(pid_str) = query.plan_id.as_deref().filter(|s| !s.is_empty()) {
        let pid = Uuid::parse_str(pid_str)
            .map_err(|_| AppError::Validation("plan_id không đúng định dạng UUID".to_string()))?;
        plan_repo::find_by_id(&state.pool, pid)
            .await?
            .ok_or_else(|| AppError::NotFound("Không tìm thấy gói Gemini Pro".to_string()))?
    } else {
        plan_repo::find_default_active(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Hiện chưa có gói Gemini Pro nào".to_string()))?
    };

    Ok(ApiResponse::ok(
        WebAppMeData {
            telegram_id: user.telegram_id,
            full_name: user.full_name,
            balance_vnd: wallet.balance_vnd,
            plan,
        },
        "Lấy thông tin tài khoản thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct SubmitActivationReq {
    pub plan_id: Uuid,
    pub email: String,
    pub password: String,
    pub two_fa_secret: String,
}

#[derive(Debug, Serialize)]
pub struct SubmitActivationRes {
    pub order_id: Uuid,
    pub order_code: String,
    pub gmail_masked: String,
    pub price_paid_vnd: i64,
    pub remaining_balance_vnd: i64,
    pub status: String,
}

/// Submit activation form (`email | pass | 2fa`) from Telegram Mini App.
pub async fn submit_activation(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Json(payload): Json<SubmitActivationReq>,
) -> Result<ApiResponse<SubmitActivationRes>, AppError> {
    let init_data = headers
        .get("x-telegram-init-data")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let tg_user =
        verify_telegram_webapp_init_data(init_data, &state.config.telegram_bot_token, 3600)?;

    let outcome = state
        .activation_service
        .submit_activation_order(
            tg_user.id,
            payload.plan_id,
            &payload.email,
            &payload.password,
            &payload.two_fa_secret,
        )
        .await?;

    // Notify Customer via Telegram
    let customer_msg = format!(
        "✅ <b>Đặt đơn kích hoạt Gemini Pro thành công!</b>\n\n\
         • Mã đơn: <code>{}</code>\n\
         • Gói: <b>{}</b>\n\
         • Gmail: <code>{}</code>\n\
         • Đã thanh toán: <b>{} đ</b>\n\
         • Số dư còn lại: <b>{} đ</b>\n\n\
         ⏳ Đơn hàng của bạn đã được tiếp nhận và sẽ sớm được xử lý. Nếu đơn lỗi, hệ thống sẽ hoàn 100% tiền về ví của bạn!",
        outcome.order.order_code,
        escape_html(&outcome.plan.name),
        escape_html(&outcome.order.gmail_masked),
        format_vnd(outcome.order.price_paid_vnd),
        format_vnd(outcome.remaining_balance_vnd),
    );
    let _ = state
        .bot
        .send_message(ChatId(outcome.user.telegram_id), customer_msg)
        .parse_mode(ParseMode::Html)
        .await;

    // Notify Admin via Telegram with exact copy-ready `email | pass | 2fa` format
    if let Some(admin_id) = state.config.admin_telegram_id {
        let admin_msg = format!(
            "🔔 <b>ĐƠN KÍCH HOẠT GEMINI PRO MỚI!</b>\n\
             • Mã đơn: <code>{}</code>\n\
             • Order ID: <code>{}</code>\n\
             • Gói: <b>{}</b> ({} đ)\n\
             • Khách: {} (ID: <code>{}</code>)\n\n\
             📋 <b>Thông tin chuẩn (chạm để copy):</b>\n\
             <code>{}</code>",
            outcome.order.order_code,
            outcome.order.id,
            escape_html(&outcome.plan.name),
            format_vnd(outcome.order.price_paid_vnd),
            escape_html(&outcome.user.full_name),
            outcome.user.telegram_id,
            escape_html(&outcome.formatted_credentials),
        );
        let _ = state
            .bot
            .send_message(ChatId(admin_id), admin_msg)
            .parse_mode(ParseMode::Html)
            .await;
    }

    Ok(ApiResponse::created(
        SubmitActivationRes {
            order_id: outcome.order.id,
            order_code: outcome.order.order_code,
            gmail_masked: outcome.order.gmail_masked,
            price_paid_vnd: outcome.order.price_paid_vnd,
            remaining_balance_vnd: outcome.remaining_balance_vnd,
            status: outcome.order.status,
        },
        "Tạo đơn kích hoạt Gemini Pro thành công",
        ctx.request_id,
    ))
}
