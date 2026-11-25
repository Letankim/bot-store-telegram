use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, header::SET_COOKIE},
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use teloxide::{
    payloads::SetWebhookSetters,
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};
use url::Url;
use uuid::Uuid;

use crate::{
    api::middleware::{ADMIN_COOKIE_NAME, RequestContext, is_admin_authenticated},
    bot::{self, messages::escape_html},
    crypto::{compute_admin_session_token, constant_time_eq_str},
    domain::{
        gemini_plan::GeminiPlan,
        partner_shop::{PartnerOrder, PartnerProduct},
        settings::AppSettings,
        wallet::{WalletTransaction, format_vnd},
    },
    error::AppError,
    repo::{
        activation_repo, partner_repo, plan_repo, settings_repo,
        user_repo::{self, AdminUserWithWallet, AdminUsersSummary},
        wallet_repo,
    },
    response::{ApiResponse, PaginationMeta},
    services::{
        activation_service::AdminOrderView,
        partner_shop_service::{self, IngestAlertOutcome, PartnerBalanceInfo, PartnerSyncSummary},
        smm_service,
    },
    state::SharedState,
};

const ADMIN_LOGIN_HTML: &str = include_str!("../../web/admin_login.html");
const ADMIN_HTML: &str = include_str!("../../web/admin.html");

/// Server-side protected `/admin`:
/// - Unauthenticated visitors ONLY receive the minimal login page HTML (zero dashboard DOM exposed).
/// - Authenticated visitors (valid HttpOnly `admin_session` cookie) receive the full dashboard.
pub async fn serve_admin_page(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Html<&'static str> {
    if is_admin_authenticated(&headers, &state.config.admin_api_key) {
        Html(ADMIN_HTML)
    } else {
        Html(ADMIN_LOGIN_HTML)
    }
}

#[derive(Debug, Deserialize)]
pub struct AdminLoginReq {
    pub api_key: String,
}

#[derive(Debug, Serialize)]
pub struct AdminAuthRes {
    pub authenticated: bool,
}

/// Authenticate admin with `ADMIN_API_KEY` and set HttpOnly `admin_session` cookie.
pub async fn admin_login(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<AdminLoginReq>,
) -> Result<Response, AppError> {
    if !constant_time_eq_str(payload.api_key.trim(), &state.config.admin_api_key) {
        return Err(AppError::Unauthorized(
            "ADMIN_API_KEY không chính xác.".to_string(),
        ));
    }

    let token = compute_admin_session_token(&state.config.admin_api_key)?;
    let secure_flag = if state.config.app_env == "production" {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{ADMIN_COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400{secure_flag}"
    );

    let mut resp = ApiResponse::ok(
        AdminAuthRes {
            authenticated: true,
        },
        "Đăng nhập quản trị thành công",
        ctx.request_id,
    )
    .into_response();

    if let Ok(val) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(SET_COOKIE, val);
    }

    Ok(resp)
}

/// Clear `admin_session` cookie and log out.
pub async fn admin_logout(Extension(ctx): Extension<RequestContext>) -> Response {
    let cookie = format!("{ADMIN_COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    let mut resp = ApiResponse::ok(
        AdminAuthRes {
            authenticated: false,
        },
        "Đã đăng xuất khỏi trang quản trị",
        ctx.request_id,
    )
    .into_response();
    if let Ok(val) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(SET_COOKIE, val);
    }
    resp
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

/// List all Gemini Pro plans (active & hidden) for Admin management.
pub async fn list_plans(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<Vec<GeminiPlan>>, AppError> {
    let plans = plan_repo::list_all_plans(&state.pool).await?;
    Ok(ApiResponse::ok(
        plans,
        "Lấy danh sách gói Gemini Pro thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct CreatePlanReq {
    pub code: String,
    pub name: String,
    pub duration_months: i32,
    pub price_vnd: i64,
    pub available_stock: i32,
    pub description_note: String,
    pub api_mode: Option<String>,
    pub form_mode: Option<String>,
    pub custom_field_label: Option<String>,
    pub display_order: Option<i32>,
}

/// Create a new product/plan from Admin Dashboard with configurable form requirements.
pub async fn create_plan(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<CreatePlanReq>,
) -> Result<ApiResponse<GeminiPlan>, AppError> {
    if payload.code.trim().is_empty() || payload.name.trim().is_empty() {
        return Err(AppError::Validation(
            "Mã gói và tên gói không được để trống.".to_string(),
        ));
    }
    if payload.price_vnd <= 0 || payload.duration_months <= 0 || payload.available_stock < 0 {
        return Err(AppError::Validation(
            "Giá, thời hạn phải lớn hơn 0 và tồn kho không được âm.".to_string(),
        ));
    }

    let form_mode = payload.form_mode.as_deref().unwrap_or("mail_pass_2fa");
    if !matches!(form_mode, "mail_pass_2fa" | "email_only" | "custom_note" | "none") {
        return Err(AppError::Validation(
            "Chế độ form chỉ hỗ trợ: mail_pass_2fa, email_only, custom_note, hoặc none."
                .to_string(),
        ));
    }

    let default_api_mode = if form_mode == "mail_pass_2fa" {
        "autopay"
    } else {
        "manual"
    };
    let api_mode = payload.api_mode.as_deref().unwrap_or(default_api_mode);
    if !matches!(api_mode, "autopay" | "payadd" | "extract" | "manual") {
        return Err(AppError::Validation(
            "Chế độ xử lý gói chỉ hỗ trợ: autopay, payadd, extract, hoặc manual.".to_string(),
        ));
    }

    let created = plan_repo::create_plan(
        &state.pool,
        &payload.code,
        &payload.name,
        payload.duration_months,
        payload.price_vnd,
        payload.available_stock,
        &payload.description_note,
        api_mode,
        form_mode,
        payload.custom_field_label.as_deref().unwrap_or(""),
        payload.display_order.unwrap_or(10),
    )
    .await?;

    Ok(ApiResponse::created(
        created,
        "Tạo gói Gemini Pro mới thành công",
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
    pub smm_api_base_url: Option<String>,
    pub smm_api_key: Option<String>,
    pub smm_auto_push: Option<bool>,
    pub smm_default_mode: Option<String>,
    pub smm_auto_refund_on_fail: Option<bool>,
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
        payload.smm_api_base_url.as_deref(),
        payload.smm_api_key.as_deref(),
        payload.smm_auto_push,
        payload.smm_default_mode.as_deref(),
        payload.smm_auto_refund_on_fail,
    )
    .await?;

    Ok(ApiResponse::ok(
        updated,
        "Lưu cấu hình hệ thống thành công (áp dụng ngay lập tức)",
        ctx.request_id,
    ))
}

/// Check live partner account info & balance from upstream Pixel API (`GET /api/v2/info`).
pub async fn smm_partner_info(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<serde_json::Value>, AppError> {
    let settings = settings_repo::get_settings(&state.pool).await?;
    let info =
        smm_service::fetch_partner_info(&settings.smm_api_base_url, &settings.smm_api_key).await?;
    Ok(ApiResponse::ok(
        info,
        "Kiểm tra kết nối API đối tác thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct PushOrderApiReq {
    pub mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PushOrderApiRes {
    pub order_id: Uuid,
    pub order_code: String,
    pub external_job_id: i64,
    pub position: Option<i64>,
    pub mode: String,
}

/// Admin manually pushes a PENDING order from the queue to the upstream Pixel API (`api.smm.io.vn`)
/// using the plan's configured `api_mode` (or optional override) and starts live 6-step Telegram progress.
pub async fn push_order_to_api(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(order_id): Path<Uuid>,
    Json(payload): Json<PushOrderApiReq>,
) -> Result<ApiResponse<PushOrderApiRes>, AppError> {
    let settings = settings_repo::get_settings(&state.pool).await?;
    let (order, creds, user_tg_id, plan_api_mode) = state
        .activation_service
        .load_decrypted_order(order_id)
        .await?;

    let can_retry_failed = order.status == "PROCESSING"
        && matches!(
            order.external_status.as_deref(),
            Some("failed") | Some("cancelled")
        );
    if order.status != "PENDING" && !can_retry_failed {
        return Err(AppError::Conflict(format!(
            "Đơn {} đang ở trạng thái {}, chỉ có thể đẩy đơn PENDING hoặc đơn PROCESSING bị API báo lỗi.",
            order.order_code, order.status
        )));
    }

    let mode = payload
        .mode
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&plan_api_mode);

    let job = smm_service::submit_pixel_job(
        &settings.smm_api_base_url,
        &settings.smm_api_key,
        &creds.email,
        &creds.password,
        &creds.two_fa_compact_upper,
        &creds.backup_codes,
        mode,
    )
    .await?;

    activation_repo::set_external_job_started(&state.pool, order.id, job.job_id, &job.mode).await?;

    if let Some(tg_id) = user_tg_id {
        smm_service::spawn_live_job_tracker(
            state.clone(),
            order.id,
            order.order_code.clone(),
            tg_id,
            creds.email,
            job.job_id,
            job.position,
            job.mode.clone(),
            settings.smm_api_base_url,
            settings.smm_api_key,
        );
    }

    Ok(ApiResponse::ok(
        PushOrderApiRes {
            order_id: order.id,
            order_code: order.order_code,
            external_job_id: job.job_id,
            position: job.position,
            mode: job.mode,
        },
        "Đã đẩy đơn sang API đối tác và bật tiến trình trực tiếp cho khách hàng",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct BatchPushOrdersReq {
    pub order_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct BatchPushItemRes {
    pub order_id: Uuid,
    pub order_code: Option<String>,
    pub ok: bool,
    pub external_job_id: Option<i64>,
    pub mode: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchPushOrdersRes {
    pub pushed: usize,
    pub failed: usize,
    pub results: Vec<BatchPushItemRes>,
}

/// Admin batch-pushes tick-selected PENDING orders to upstream Pixel API using each order's own plan `api_mode`.
pub async fn batch_push_orders_to_api(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<BatchPushOrdersReq>,
) -> Result<ApiResponse<BatchPushOrdersRes>, AppError> {
    if payload.order_ids.is_empty() {
        return Err(AppError::Validation(
            "Vui lòng tick chọn ít nhất 1 đơn hàng để đẩy qua API.".to_string(),
        ));
    }

    let settings = settings_repo::get_settings(&state.pool).await?;
    if settings.smm_api_key.trim().is_empty() {
        return Err(AppError::Validation(
            "Chưa cấu hình API Key đối tác trong mục Cấu hình hệ thống.".to_string(),
        ));
    }

    let mut pushed = 0usize;
    let mut failed = 0usize;
    let mut results = Vec::with_capacity(payload.order_ids.len());

    for order_id in payload.order_ids {
        let loaded = state
            .activation_service
            .load_decrypted_order(order_id)
            .await;

        let (order, creds, user_tg_id, plan_api_mode) = match loaded {
            Ok(tuple) => tuple,
            Err(err) => {
                failed += 1;
                results.push(BatchPushItemRes {
                    order_id,
                    order_code: None,
                    ok: false,
                    external_job_id: None,
                    mode: None,
                    error: Some(err.to_string()),
                });
                continue;
            }
        };

        if order.status != "PENDING" {
            failed += 1;
            results.push(BatchPushItemRes {
                order_id,
                order_code: Some(order.order_code),
                ok: false,
                external_job_id: None,
                mode: Some(plan_api_mode),
                error: Some(format!("Trạng thái hiện tại là {}", order.status)),
            });
            continue;
        }

        match smm_service::submit_pixel_job(
            &settings.smm_api_base_url,
            &settings.smm_api_key,
            &creds.email,
            &creds.password,
            &creds.two_fa_compact_upper,
            &creds.backup_codes,
            &plan_api_mode,
        )
        .await
        {
            Ok(job) => {
                let _ = activation_repo::set_external_job_started(
                    &state.pool,
                    order.id,
                    job.job_id,
                    &job.mode,
                )
                .await;

                if let Some(tg_id) = user_tg_id {
                    smm_service::spawn_live_job_tracker(
                        state.clone(),
                        order.id,
                        order.order_code.clone(),
                        tg_id,
                        creds.email,
                        job.job_id,
                        job.position,
                        job.mode.clone(),
                        settings.smm_api_base_url.clone(),
                        settings.smm_api_key.clone(),
                    );
                }

                pushed += 1;
                results.push(BatchPushItemRes {
                    order_id: order.id,
                    order_code: Some(order.order_code),
                    ok: true,
                    external_job_id: Some(job.job_id),
                    mode: Some(job.mode),
                    error: None,
                });
            }
            Err(err) => {
                failed += 1;
                results.push(BatchPushItemRes {
                    order_id: order.id,
                    order_code: Some(order.order_code),
                    ok: false,
                    external_job_id: None,
                    mode: Some(plan_api_mode),
                    error: Some(err.to_string()),
                });
            }
        }
    }

    let message = format!("Đã đẩy thành công {pushed} đơn, lỗi {failed} đơn.");
    Ok(ApiResponse::ok(
        BatchPushOrdersRes {
            pushed,
            failed,
            results,
        },
        message,
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

#[derive(Debug, Serialize)]
pub struct SyncOrderLinkRes {
    pub order_id: Uuid,
    pub order_code: String,
    pub external_job_id: i64,
    pub extract_link: Option<String>,
    pub sent_to_user: bool,
}

/// Sync `extract_link` from upstream Pixel API for an existing order and resend it to the customer on Telegram.
pub async fn sync_and_resend_order_link(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(order_id): Path<Uuid>,
) -> Result<ApiResponse<SyncOrderLinkRes>, AppError> {
    let settings = settings_repo::get_settings(&state.pool).await?;
    let order = activation_repo::find_by_id(&state.pool, order_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Không tìm thấy đơn hàng".to_string()))?;

    let job_id = order.external_job_id.ok_or_else(|| {
        AppError::Validation("Đơn hàng này chưa được đẩy qua API đối tác (chưa có Job ID).".to_string())
    })?;

    let (ext_status, ext_step, maybe_link) =
        smm_service::fetch_job_status(&settings.smm_api_base_url, &settings.smm_api_key, job_id)
            .await?;

    let Some(link) = maybe_link else {
        return Err(AppError::NotFound(format!(
            "Job #{job_id} (trạng thái: {ext_status}) hiện không có link ưu đãi."
        )));
    };

    activation_repo::update_external_progress(
        &state.pool,
        order.id,
        &ext_status,
        ext_step.as_deref(),
        Some(&link),
    )
    .await?;

    let mut sent_to_user = false;
    if let Ok(Some(user)) = user_repo::find_by_id(&state.pool, order.user_id).await {
        let msg = format!(
            "🔗 <b>Link ưu đãi cho đơn hàng của bạn!</b>\n\n\
             • Mã đơn: <code>{}</code>\n\
             • Gmail: <code>{}</code>\n\
             • Link ưu đãi:\n<code>{}</code>",
            order.order_code,
            escape_html(&order.gmail_masked),
            escape_html(&link),
        );
        let mut req = state
            .bot
            .send_message(ChatId(user.telegram_id), msg)
            .parse_mode(ParseMode::Html);
        if let Ok(parsed_url) = Url::parse(&link) {
            req = req.reply_markup(InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::url("🔗 Mở link ưu đãi ngay", parsed_url),
            ]]));
        }
        if req.await.is_ok() {
            sent_to_user = true;
        }
    }

    Ok(ApiResponse::ok(
        SyncOrderLinkRes {
            order_id: order.id,
            order_code: order.order_code,
            external_job_id: job_id,
            extract_link: Some(link),
            sent_to_user,
        },
        "Đã đồng bộ link ưu đãi và gửi bù cho khách hàng qua Telegram!",
        ctx.request_id,
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
            "🎉 <b>Đơn kích hoạt Gemini Pro đã hoàn tất!</b>\n\n\
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

    let new_bal = outcome.refunded_balance_vnd.unwrap_or(0);
    let reason_clean = payload.reason.trim();
    let plan_name = outcome
        .plan_name
        .as_deref()
        .unwrap_or("Gemini Pro");

    let customer_msg = format!(
        "↩️ <b>Hoàn tiền 100% đơn hàng lỗi!</b>\n\n\
         • Mã đơn: <code>{}</code>\n\
         • Gói: <b>{}</b>\n\
         • Gmail: <code>{}</code>\n\
         • Số tiền hoàn lại ví: <b>+{} đ</b>\n\
         • Số dư ví hiện tại: <b>{} đ</b>\n\
         • Lý do: <i>{}</i>\n\n\
         📘 Vui lòng kiểm tra lại hướng dẫn (/guide) trước khi đặt lại đơn mới nhé!",
        outcome.order.order_code,
        escape_html(plan_name),
        escape_html(&outcome.order.gmail_masked),
        format_vnd(outcome.order.price_paid_vnd),
        format_vnd(new_bal),
        escape_html(reason_clean),
    );

    let mut sent_to_customer = false;
    if let Some(tg_id) = outcome.user_telegram_id {
        if state
            .bot
            .send_message(ChatId(tg_id), &customer_msg)
            .parse_mode(ParseMode::Html)
            .await
            .is_ok()
        {
            sent_to_customer = true;
        }
    }

    if let Some(admin_id) = state.config.admin_telegram_id {
        let cust_name = outcome
            .user_full_name
            .as_deref()
            .unwrap_or("Khách hàng");
        let cust_tg_str = outcome
            .user_telegram_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        let delivery_status = if sent_to_customer {
            "✅ Đã gửi tới khách thành công"
        } else {
            "⚠️ Chưa gửi được tới khách (khách có thể đã chặn bot)"
        };

        let admin_msg = format!(
            "🛡️ <b>[ADMIN] ĐÃ HOÀN TIỀN ĐƠN HÀNG</b>\n\n\
             • Mã đơn: <code>{}</code>\n\
             • Khách hàng: <b>{}</b> (ID: <code>{}</code>)\n\
             • Gói: <b>{}</b>\n\
             • Hoàn tiền: <b>+{} đ</b> (Số dư mới: <b>{} đ</b>)\n\
             • Trạng thái gửi khách: {}\n\n\
             📩 <b>Nội dung khách đã nhận:</b>\n\
             <blockquote>{}</blockquote>",
            outcome.order.order_code,
            escape_html(cust_name),
            cust_tg_str,
            escape_html(plan_name),
            format_vnd(outcome.order.price_paid_vnd),
            format_vnd(new_bal),
            delivery_status,
            customer_msg,
        );
        let _ = state
            .bot
            .send_message(ChatId(admin_id), admin_msg)
            .parse_mode(ParseMode::Html)
            .await;
    }

    Ok(ApiResponse::ok(
        OrderStatusRes {
            order_id: outcome.order.id,
            order_code: outcome.order.order_code,
            status: outcome.order.status,
        },
        "Đã hoàn tiền 100% và gửi thông báo cho cả khách hàng & Admin",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePlanReq {
    pub name: Option<String>,
    pub duration_months: Option<i32>,
    pub price_vnd: Option<i64>,
    pub available_stock: Option<i32>,
    pub sold_count: Option<i32>,
    pub description_note: Option<String>,
    pub api_mode: Option<String>,
    pub form_mode: Option<String>,
    pub custom_field_label: Option<String>,
    pub display_order: Option<i32>,
    pub is_active: Option<bool>,
    pub is_maintenance: Option<bool>,
    pub maintenance_note: Option<String>,
    pub notify_restock: Option<bool>,
    pub notify_maintenance: Option<bool>,
}

/// Update full details of a product/plan (name, price, stock, sold count, description, api_mode, form_mode, status).
/// Automatically broadcasts restock or maintenance status notifications to all active users when enabled.
pub async fn update_plan(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(plan_id): Path<Uuid>,
    Json(payload): Json<UpdatePlanReq>,
) -> Result<ApiResponse<GeminiPlan>, AppError> {
    if let Some(mode) = payload.api_mode.as_deref() {
        if !matches!(mode, "autopay" | "payadd" | "extract" | "manual") {
            return Err(AppError::Validation(
                "Chế độ xử lý gói chỉ hỗ trợ: autopay, payadd, extract, hoặc manual.".to_string(),
            ));
        }
    }
    if let Some(fm) = payload.form_mode.as_deref() {
        if !matches!(fm, "mail_pass_2fa" | "email_only" | "custom_note" | "none") {
            return Err(AppError::Validation(
                "Chế độ form chỉ hỗ trợ: mail_pass_2fa, email_only, custom_note, hoặc none."
                    .to_string(),
            ));
        }
    }

    let previous = plan_repo::find_by_id(&state.pool, plan_id).await?;
    let previous_stock = previous.as_ref().map(|p| p.available_stock).unwrap_or(0);
    let previous_maintenance = previous.as_ref().map(|p| p.is_maintenance).unwrap_or(false);

    let updated = plan_repo::update_plan_full(
        &state.pool,
        plan_id,
        payload.name.as_deref(),
        payload.duration_months,
        payload.price_vnd,
        payload.available_stock,
        payload.sold_count,
        payload.description_note.as_deref(),
        payload.api_mode.as_deref(),
        payload.form_mode.as_deref(),
        payload.custom_field_label.as_deref(),
        payload.display_order,
        payload.is_active,
        payload.is_maintenance,
        payload.maintenance_note.as_deref(),
    )
    .await?;

    let maintenance_toggled = previous_maintenance != updated.is_maintenance;
    let should_notify_maintenance = maintenance_toggled
        && updated.is_active
        && payload.notify_maintenance.unwrap_or(true);

    let added_qty = updated.available_stock - previous_stock;
    let should_notify_restock = !should_notify_maintenance
        && added_qty > 0
        && updated.is_active
        && !updated.is_maintenance
        && payload.notify_restock.unwrap_or(true);

    if should_notify_maintenance {
        let state_bg = state.clone();
        let plan_bg = updated.clone();
        let is_starting_maint = updated.is_maintenance;
        tokio::spawn(async move {
            let Ok(tg_ids) = user_repo::list_all_active_telegram_ids(&state_bg.pool).await else {
                return;
            };
            let total = tg_ids.len();
            let (text, kb) = if is_starting_maint {
                (
                    bot::messages::maintenance_started_broadcast_message(&plan_bg),
                    bot::keyboards::maintenance_broadcast_keyboard(),
                )
            } else {
                (
                    bot::messages::maintenance_ended_broadcast_message(&plan_bg),
                    bot::keyboards::restock_broadcast_keyboard(&plan_bg.id.to_string()),
                )
            };

            let mut sent_ok = 0usize;
            for tg_id in tg_ids {
                if state_bg
                    .bot
                    .send_message(ChatId(tg_id), &text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(kb.clone())
                    .await
                    .is_ok()
                {
                    sent_ok += 1;
                }
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
            tracing::info!(
                plan_code = %plan_bg.code,
                is_starting_maint,
                sent_ok,
                total,
                "Finished maintenance status broadcast notification"
            );
        });
    } else if should_notify_restock {
        let state_bg = state.clone();
        let plan_bg = updated.clone();
        tokio::spawn(async move {
            let Ok(tg_ids) = user_repo::list_all_active_telegram_ids(&state_bg.pool).await else {
                return;
            };
            let text = bot::messages::restock_broadcast_message(&plan_bg, added_qty);
            let kb = bot::keyboards::restock_broadcast_keyboard(&plan_bg.id.to_string());
            let mut sent_ok = 0usize;
            for tg_id in tg_ids {
                if state_bg
                    .bot
                    .send_message(ChatId(tg_id), &text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(kb.clone())
                    .await
                    .is_ok()
                {
                    sent_ok += 1;
                }
                // Pace at ~25 msg/sec to stay safely below Telegram's 30 msg/sec broadcast limit
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
            tracing::info!(
                plan_code = %plan_bg.code,
                added_qty,
                sent_ok,
                "Finished restock broadcast notification"
            );
        });
    }

    let msg = if should_notify_maintenance && updated.is_maintenance {
        "Đã bật bảo trì gói và đang gửi thông báo bảo trì tới tất cả khách hàng!".to_string()
    } else if should_notify_maintenance && !updated.is_maintenance {
        "Đã tắt bảo trì gói và đang gửi thông báo mở bán lại tới tất cả khách hàng!".to_string()
    } else if should_notify_restock {
        format!(
            "Đã cập nhật gói (+{added_qty} tồn kho) và đang gửi thông báo lên kho tới tất cả người dùng!"
        )
    } else {
        "Cập nhật thông tin gói Gemini Pro thành công".to_string()
    };

    Ok(ApiResponse::ok(updated, msg, ctx.request_id))
}

#[derive(Debug, Deserialize)]
pub struct BroadcastReq {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct BroadcastRes {
    pub target_users: usize,
}

/// Admin manually broadcasts a maintenance or general announcement to all active Telegram users.
pub async fn broadcast_announcement(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<BroadcastReq>,
) -> Result<ApiResponse<BroadcastRes>, AppError> {
    let title = payload.title.trim();
    let content = payload.content.trim();
    if title.is_empty() || content.is_empty() {
        return Err(AppError::Validation(
            "Vui lòng nhập đầy đủ tiêu đề và nội dung thông báo.".to_string(),
        ));
    }

    let tg_ids = user_repo::list_all_active_telegram_ids(&state.pool).await?;
    let total = tg_ids.len();

    let formatted = format!(
        "<b>{}</b>\n\
         ══════════════════════════\n\n\
         {}",
        escape_html(title),
        escape_html(content),
    );

    let state_bg = state.clone();
    tokio::spawn(async move {
        let kb = bot::keyboards::maintenance_broadcast_keyboard();
        let mut sent_ok = 0usize;
        for tg_id in tg_ids {
            if state_bg
                .bot
                .send_message(ChatId(tg_id), &formatted)
                .parse_mode(ParseMode::Html)
                .reply_markup(kb.clone())
                .await
                .is_ok()
            {
                sent_ok += 1;
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        if let Some(admin_id) = state_bg.config.admin_telegram_id {
            let report = format!(
                "📢 <b>[ADMIN] HOÀN TẤT GỬI THÔNG BÁO HÀNG LOẠT</b>\n\n\
                 • Đã gửi thành công: <b>{sent_ok} / {total}</b> người dùng\n\n\
                 📩 <b>Nội dung đã gửi:</b>\n\
                 <blockquote>{formatted}</blockquote>"
            );
            let _ = state_bg
                .bot
                .send_message(ChatId(admin_id), report)
                .parse_mode(ParseMode::Html)
                .await;
        }
    });

    Ok(ApiResponse::ok(
        BroadcastRes { target_users: total },
        format!("Đang gửi thông báo tới {total} người dùng hoạt động!"),
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

#[derive(Debug, Deserialize)]
pub struct ListUsersQuery {
    pub search: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ListUsersRes {
    pub summary: AdminUsersSummary,
    pub users: Vec<AdminUserWithWallet>,
}

/// List all users with global summary stats, wallet balances, order counts & pagination.
pub async fn list_users(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<ListUsersQuery>,
) -> Result<ApiResponse<ListUsersRes>, AppError> {
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(20).clamp(1, 100);
    let offset = i64::from((page - 1) * per_page);

    let summary = user_repo::get_users_summary(&state.pool).await?;
    let (users, total) = user_repo::list_users_with_wallets(
        &state.pool,
        q.search.as_deref(),
        i64::from(per_page),
        offset,
    )
    .await?;

    let total_pages = ((total as f64) / (per_page as f64)).ceil().max(1.0) as u32;

    Ok(ApiResponse::ok_paginated(
        ListUsersRes { summary, users },
        "Lấy danh sách người dùng thành công",
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
pub struct AdjustWalletReq {
    /// Positive to credit, negative to debit.
    pub delta_vnd: i64,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdjustWalletRes {
    pub user_id: Uuid,
    pub balance_vnd: i64,
}

/// Admin credit or debit any user's wallet with immutable ledger entry + Telegram notification.
pub async fn adjust_user_wallet(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<AdjustWalletReq>,
) -> Result<ApiResponse<AdjustWalletRes>, AppError> {
    if payload.delta_vnd == 0 {
        return Err(AppError::Validation("Số tiền điều chỉnh phải khác 0.".to_string()));
    }

    let user = user_repo::find_by_id(&state.pool, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Không tìm thấy người dùng".to_string()))?;

    let mut tx = state.pool.begin().await?;
    let wallet = wallet_repo::lock_by_user_id(&mut tx, user.id).await?;
    let idem_key = format!("admin_adj_{}", Uuid::new_v4().simple());
    let desc = payload
        .description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Admin điều chỉnh số dư ví")
        .to_string();

    let updated = if payload.delta_vnd > 0 {
        wallet_repo::credit_wallet(
            &mut tx,
            &wallet,
            payload.delta_vnd,
            false,
            "ADMIN_ADJUST",
            &idem_key,
            &desc,
        )
        .await?
    } else {
        wallet_repo::debit_wallet(
            &mut tx,
            &wallet,
            payload.delta_vnd.abs(),
            "ADMIN_ADJUST",
            &idem_key,
            &desc,
        )
        .await?
    };

    tx.commit().await?;

    // Notify user on Telegram
    let sign_txt = if payload.delta_vnd > 0 {
        format!("+{} đ", format_vnd(payload.delta_vnd))
    } else {
        format!("-{} đ", format_vnd(payload.delta_vnd.abs()))
    };
    let tg_msg = format!(
        "💰 <b>Cập nhật số dư ví từ quản trị viên</b>\n\n\
         • Biến động: <b>{}</b>\n\
         • Số dư hiện tại: <b>{} đ</b>\n\
         • Ghi chú: <i>{}</i>",
        sign_txt,
        format_vnd(updated.balance_vnd),
        escape_html(&desc)
    );
    if user.telegram_id > 0 {
        let _ = state
            .bot
            .send_message(ChatId(user.telegram_id), tg_msg)
            .parse_mode(ParseMode::Html)
            .await;
    }

    Ok(ApiResponse::ok(
        AdjustWalletRes {
            user_id: user.id,
            balance_vnd: updated.balance_vnd,
        },
        "Điều chỉnh số dư ví thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct ToggleBanReq {
    pub is_banned: bool,
}

#[derive(Debug, Serialize)]
pub struct ToggleBanRes {
    pub user_id: Uuid,
    pub is_banned: bool,
}

/// Ban or unban a user account.
pub async fn toggle_user_ban(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<ToggleBanReq>,
) -> Result<ApiResponse<ToggleBanRes>, AppError> {
    let updated = user_repo::set_user_banned(&state.pool, user_id, payload.is_banned).await?;
    if updated.is_banned {
        let _ = user_repo::delete_all_user_sessions(&state.pool, user_id).await;
    }
    Ok(ApiResponse::ok(
        ToggleBanRes {
            user_id: updated.id,
            is_banned: updated.is_banned,
        },
        if updated.is_banned {
            "Đã khóa tài khoản người dùng"
        } else {
            "Đã mở khóa tài khoản người dùng"
        },
        ctx.request_id,
    ))
}

/// Fetch recent immutable ledger transactions for a user's wallet.
pub async fn list_user_transactions(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(user_id): Path<Uuid>,
) -> Result<ApiResponse<Vec<WalletTransaction>>, AppError> {
    let wallet = wallet_repo::get_by_user_id(&state.pool, user_id).await?;
    let txs = wallet_repo::list_recent_transactions(&state.pool, wallet.id, 20).await?;
    Ok(ApiResponse::ok(
        txs,
        "Lấy lịch sử giao dịch ví thành công",
        ctx.request_id,
    ))
}

// =============================================================================
// Partner Shop API Management Endpoints (`/api/v1/admin/partner-shop/*`)
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct UpdatePartnerShopSettingsReq {
    pub partner_shop_base_url: String,
    pub partner_shop_api_key: Option<String>,
    pub partner_shop_enabled: bool,
    pub partner_shop_markup_vnd: i64,
    pub partner_shop_sync_interval_secs: i32,
    pub partner_shop_notify_changes: bool,
}

/// Update Partner Shop API credentials, profit markup (default +30,000 VND) & sync settings.
pub async fn update_partner_shop_settings(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<UpdatePartnerShopSettingsReq>,
) -> Result<ApiResponse<AppSettings>, AppError> {
    let updated = settings_repo::update_partner_shop_settings(
        &state.pool,
        &payload.partner_shop_base_url,
        payload.partner_shop_api_key.as_deref(),
        payload.partner_shop_enabled,
        payload.partner_shop_markup_vnd,
        payload.partner_shop_sync_interval_secs,
        payload.partner_shop_notify_changes,
    )
    .await?;

    Ok(ApiResponse::ok(
        updated,
        "Đã lưu cấu hình Shop đối tác & cập nhật giá bán theo mức lời mới!",
        ctx.request_id,
    ))
}

/// Check live Partner Shop wallet balance (`GET /api/partner-api/balance`).
pub async fn get_partner_shop_balance(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<PartnerBalanceInfo>, AppError> {
    let settings = settings_repo::get_settings(&state.pool).await?;
    let info = partner_shop_service::fetch_partner_balance(
        &settings.partner_shop_base_url,
        &settings.partner_shop_api_key,
    )
    .await?;

    Ok(ApiResponse::ok(
        info,
        "Kiểm tra số dư Shop đối tác thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct TriggerPartnerSyncReq {
    pub notify_changes: Option<bool>,
}

/// Trigger an immediate sync of all products, prices & stock from Partner Shop API.
pub async fn sync_partner_shop_now(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<TriggerPartnerSyncReq>,
) -> Result<ApiResponse<PartnerSyncSummary>, AppError> {
    let notify = payload.notify_changes.unwrap_or(true);
    let summary = partner_shop_service::sync_partner_products(&state, notify).await?;
    let msg = format!(
        "Đồng bộ xong {} sản phẩm (mới: {}, đổi giá: {}, lên kho: {}).",
        summary.total_synced,
        summary.new_count,
        summary.price_changed_count,
        summary.restocked_count
    );
    Ok(ApiResponse::ok(summary, msg, ctx.request_id))
}

/// List all synced Partner Shop products with cost price, profit markup & sell price for Admin.
pub async fn list_partner_shop_products(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<Vec<PartnerProduct>>, AppError> {
    let items = partner_repo::list_all_products(&state.pool).await?;
    Ok(ApiResponse::ok(
        items,
        "Lấy danh sách sản phẩm Shop đối tác thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePartnerProductReq {
    /// When `Some(Some(val))` -> set custom markup; `Some(None)` -> reset to global default markup; `None` -> unchanged.
    #[serde(default)]
    pub set_custom_markup: bool,
    pub custom_markup_vnd: Option<i64>,
    pub is_active: Option<bool>,
    pub is_maintenance: Option<bool>,
    pub maintenance_note: Option<String>,
}

/// Update custom profit markup, visibility, or maintenance status of a synced partner product.
pub async fn update_partner_shop_product(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Path(product_id): Path<Uuid>,
    Json(payload): Json<UpdatePartnerProductReq>,
) -> Result<ApiResponse<PartnerProduct>, AppError> {
    let settings = settings_repo::get_settings(&state.pool).await?;
    let updated = partner_repo::update_product_admin(
        &state.pool,
        product_id,
        settings.partner_shop_markup_vnd,
        payload.set_custom_markup,
        payload.custom_markup_vnd,
        payload.is_active,
        payload.is_maintenance,
        payload.maintenance_note.as_deref(),
    )
    .await?;

    Ok(ApiResponse::ok(
        updated,
        "Cập nhật sản phẩm Shop đối tác thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct ListPartnerOrdersQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// List Partner Shop purchase orders with revenue, partner cost & net profit.
pub async fn list_partner_shop_orders(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<ListPartnerOrdersQuery>,
) -> Result<ApiResponse<Vec<PartnerOrder>>, AppError> {
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(20).clamp(1, 100);
    let offset = i64::from((page - 1) * per_page);
    let (orders, total) =
        partner_repo::list_orders_for_admin(&state.pool, i64::from(per_page), offset).await?;
    let total_pages = ((total as f64) / (per_page as f64)).ceil() as u32;

    Ok(ApiResponse::ok_paginated(
        orders,
        "Lấy danh sách đơn Shop đối tác thành công",
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
pub struct IngestPartnerMessageReq {
    pub text: String,
    pub notify_customers: Option<bool>,
}

/// Ingest a raw partner Telegram bot restock/price alert message (called automatically by
/// the 24/7 Telethon auto-sync listener or manually), apply profit markup, upsert the product,
/// and broadcast price/stock updates to customers.
pub async fn ingest_partner_message(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<IngestPartnerMessageReq>,
) -> Result<ApiResponse<IngestAlertOutcome>, AppError> {
    if payload.text.trim().is_empty() {
        return Err(AppError::Validation(
            "Nội dung tin nhắn không được để trống.".to_string(),
        ));
    }
    let notify = payload.notify_customers.unwrap_or(true);
    let outcome =
        partner_shop_service::ingest_partner_alert_message(&state, &payload.text, notify).await?;
    let msg = if outcome.created {
        format!(
            "Đã tự động tạo sản phẩm mới '{}' với giá bán {}đ (kho: {}).",
            outcome.plan.name,
            format_vnd(outcome.sell_price_vnd),
            outcome.plan.available_stock
        )
    } else {
        format!(
            "Đã tự động cập nhật '{}' — giá bán {}đ (kho: {}).",
            outcome.plan.name,
            format_vnd(outcome.sell_price_vnd),
            outcome.plan.available_stock
        )
    };
    Ok(ApiResponse::ok(outcome, msg, ctx.request_id))
}
