use std::time::{Duration, Instant};

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header::SET_COOKIE},
    response::{Html, IntoResponse, Response},
};
use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use teloxide::{prelude::*, types::ParseMode};
use uuid::Uuid;

use crate::{
    api::middleware::{
        RequestContext, WEB_SESSION_COOKIE_NAME, WebAuthUser, extract_cookie,
    },
    bot::messages::escape_html,
    crypto::{
        generate_session_token, hash_password_argon2, sha256_hex, verify_password_argon2,
    },
    domain::{
        gemini_plan::GeminiPlan,
        partner_shop::PublicPartnerProduct,
        wallet::{WalletTransaction, format_vnd},
    },
    error::AppError,
    repo::{activation_repo, partner_repo, plan_repo, settings_repo, user_repo, wallet_repo},
    response::ApiResponse,
    services::{
        partner_shop_service,
        payos_service::CheckPaymentStatus,
        referral_service::{self, ReferralSummary},
    },
    state::SharedState,
};

const STORE_HTML: &str = include_str!("../../web/store.html");

const AUTH_RATE_LIMIT_MAX: usize = 10;
const AUTH_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(300); // 5 minutes
const SESSION_TTL_DAYS: i64 = 7;

/// Serve public Web Storefront (`GET /`) for customers without Telegram.
pub async fn serve_store_page() -> Html<&'static str> {
    Html(STORE_HTML)
}

fn client_ip_key(headers: &HeaderMap, action: &str, email_lower: &str) -> String {
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown");
    format!("{action}:{ip}:{email_lower}")
}

async fn check_auth_rate_limit(state: &SharedState, key: &str) -> Result<(), AppError> {
    let now = Instant::now();
    let mut map = state.web_auth_rate_limits.write().await;

    // Periodic lightweight cleanup to bound memory
    if map.len() > 2000 {
        map.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < AUTH_RATE_LIMIT_WINDOW);
            !times.is_empty()
        });
    }

    let entries = map.entry(key.to_string()).or_default();
    entries.retain(|t| now.duration_since(*t) < AUTH_RATE_LIMIT_WINDOW);
    if entries.len() >= AUTH_RATE_LIMIT_MAX {
        return Err(AppError::TooManyRequests(
            "Bạn đã thao tác quá nhiều lần. Vui lòng thử lại sau 5 phút.".to_string(),
        ));
    }
    entries.push(now);
    Ok(())
}

fn build_session_cookie(raw_token: &str, is_production: bool) -> String {
    let secure_flag = if is_production { "; Secure" } else { "" };
    let max_age = SESSION_TTL_DAYS * 86_400;
    format!(
        "{WEB_SESSION_COOKIE_NAME}={raw_token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}{secure_flag}"
    )
}

fn build_clear_session_cookie(is_production: bool) -> String {
    let secure_flag = if is_production { "; Secure" } else { "" };
    format!("{WEB_SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{secure_flag}")
}

fn require_csrf_xhr(headers: &HeaderMap) -> Result<(), AppError> {
    let ok = headers
        .get("x-requested-with")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("XMLHttpRequest"))
        .unwrap_or(false);
    if !ok {
        return Err(AppError::Forbidden(
            "Thiếu header bảo vệ CSRF hợp lệ.".to_string(),
        ));
    }
    Ok(())
}

fn is_valid_email(email: &str) -> bool {
    let e = email.trim();
    if e.len() < 5 || e.len() > 250 || e.contains(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

#[derive(Debug, Serialize)]
pub struct PublicStoreSettings {
    pub support_handle: String,
    pub min_deposit_vnd: i64,
    pub quick_deposit_amounts: Vec<i64>,
    pub referral_reward_vnd: i64,
    pub referred_bonus_vnd: i64,
    pub referral_min_first_deposit_vnd: i64,
    pub guide_url: String,
}

#[derive(Debug, Serialize)]
pub struct WebCatalogRes {
    pub plans: Vec<GeminiPlan>,
    pub partner_products: Vec<PublicPartnerProduct>,
    pub settings: PublicStoreSettings,
}

/// Public catalog endpoint (`GET /api/v1/web/catalog`) — zero sensitive fields exposed.
pub async fn get_catalog(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
) -> Result<ApiResponse<WebCatalogRes>, AppError> {
    let plans = plan_repo::list_active_plans(&state.pool).await?;
    let partner_products: Vec<PublicPartnerProduct> = partner_repo::list_active_products(&state.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(PublicPartnerProduct::from)
        .collect();
    let s = settings_repo::get_settings(&state.pool).await?;

    let public_settings = PublicStoreSettings {
        support_handle: s.support_handle.clone(),
        min_deposit_vnd: s.min_deposit_vnd,
        quick_deposit_amounts: s.parsed_quick_amounts(),
        referral_reward_vnd: s.referral_reward_vnd,
        referred_bonus_vnd: s.referred_bonus_vnd,
        referral_min_first_deposit_vnd: s.referral_min_first_deposit_vnd,
        guide_url: state.config.guide_url(None),
    };

    Ok(ApiResponse::ok(
        WebCatalogRes {
            plans,
            partner_products,
            settings: public_settings,
        },
        "Tải danh sách gói dịch vụ thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct WebRegisterReq {
    pub full_name: String,
    pub email: String,
    pub password: String,
    pub ref_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WebAuthProfile {
    pub id: Uuid,
    pub email: String,
    pub full_name: String,
    pub ref_code: String,
    pub balance_vnd: i64,
    pub total_deposited_vnd: i64,
}

/// Register a new Web-only customer account with Argon2id password hashing + HttpOnly opaque session.
pub async fn web_register(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Json(payload): Json<WebRegisterReq>,
) -> Result<Response, AppError> {
    require_csrf_xhr(&headers)?;

    let email_lower = payload.email.trim().to_lowercase();
    let full_name = payload.full_name.trim().to_string();
    let password = payload.password;

    let rl_key = client_ip_key(&headers, "register", &email_lower);
    check_auth_rate_limit(&state, &rl_key).await?;

    if full_name.len() < 2 || full_name.len() > 100 {
        return Err(AppError::Validation(
            "Họ tên cần từ 2 đến 100 ký tự.".to_string(),
        ));
    }
    if !is_valid_email(&email_lower) {
        return Err(AppError::Validation(
            "Địa chỉ email không hợp lệ.".to_string(),
        ));
    }
    if password.len() < 8 || password.len() > 128 {
        return Err(AppError::Validation(
            "Mật khẩu cần tối thiểu 8 ký tự (tối đa 128 ký tự).".to_string(),
        ));
    }

    // Offload Argon2id hashing to blocking thread pool so async runtime stays responsive
    let password_hash = tokio::task::spawn_blocking(move || hash_password_argon2(&password))
        .await
        .map_err(|e| AppError::Internal(format!("Join error hashing password: {e}")))??;

    // Optional referral code lookup
    let mut referrer = None;
    if let Some(code) = payload
        .ref_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let clean_code = code.strip_prefix("ref_").unwrap_or(code);
        if let Ok(Some(found)) = user_repo::find_by_ref_code(&state.pool, clean_code).await {
            referrer = Some(found);
        }
    }

    let settings = settings_repo::get_settings(&state.pool).await.ok();
    let reward_vnd = settings
        .as_ref()
        .map(|s| s.referral_reward_vnd)
        .unwrap_or(state.config.referral_reward_vnd);
    let referred_bonus_vnd = settings
        .as_ref()
        .map(|s| s.referred_bonus_vnd)
        .unwrap_or(2_000);

    let mut tx = state.pool.begin().await?;
    let user = user_repo::create_web_user_with_wallet(
        &mut tx,
        &email_lower,
        &password_hash,
        &full_name,
        referrer.as_ref().map(|r| r.id),
        reward_vnd,
        referred_bonus_vnd,
    )
    .await?;
    tx.commit().await?;

    // Create opaque server-side session
    let raw_token = generate_session_token();
    let token_hash = sha256_hex(&raw_token);
    let expires_at = Utc::now() + ChronoDuration::days(SESSION_TTL_DAYS);
    user_repo::create_web_session(&state.pool, user.id, &token_hash, expires_at).await?;

    let is_prod = state.config.app_env == "production";
    let cookie = build_session_cookie(&raw_token, is_prod);

    let mut resp = ApiResponse::created(
        WebAuthProfile {
            id: user.id,
            email: email_lower,
            full_name: user.full_name,
            ref_code: user.ref_code,
            balance_vnd: 0,
            total_deposited_vnd: 0,
        },
        "Đăng ký tài khoản thành công",
        ctx.request_id,
    )
    .into_response();

    if let Ok(val) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(SET_COOKIE, val);
    }
    Ok(resp)
}

#[derive(Debug, Deserialize)]
pub struct WebLoginReq {
    pub email: String,
    pub password: String,
}

/// Login with email + Argon2id password verification + rate limiting + HttpOnly opaque session.
pub async fn web_login(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    Json(payload): Json<WebLoginReq>,
) -> Result<Response, AppError> {
    require_csrf_xhr(&headers)?;

    let email_lower = payload.email.trim().to_lowercase();
    let rl_key = client_ip_key(&headers, "login", &email_lower);
    check_auth_rate_limit(&state, &rl_key).await?;

    let invalid_err = || AppError::Unauthorized("Email hoặc mật khẩu không chính xác.".to_string());

    let Some(user) = user_repo::find_by_email(&state.pool, &email_lower).await? else {
        // Run a dummy Argon2 verify to mitigate timing-based user enumeration
        let pwd = payload.password;
        let _ = tokio::task::spawn_blocking(move || {
            verify_password_argon2(
                &pwd,
                "$argon2id$v=19$m=19456,t=2,p=1$AAAAAAAAAAAAAAAAAAAAAA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            )
        })
        .await;
        return Err(invalid_err());
    };

    let Some(stored_hash) = user.password_hash.clone() else {
        return Err(invalid_err());
    };

    let pwd = payload.password;
    let verified = tokio::task::spawn_blocking(move || verify_password_argon2(&pwd, &stored_hash))
        .await
        .map_err(|e| AppError::Internal(format!("Join error verifying password: {e}")))?;

    if !verified {
        return Err(invalid_err());
    }

    if user.is_banned {
        return Err(AppError::Forbidden(
            "Tài khoản của bạn đã bị khóa. Vui lòng liên hệ hỗ trợ.".to_string(),
        ));
    }

    let wallet = wallet_repo::get_by_user_id(&state.pool, user.id).await?;

    let raw_token = generate_session_token();
    let token_hash = sha256_hex(&raw_token);
    let expires_at = Utc::now() + ChronoDuration::days(SESSION_TTL_DAYS);
    user_repo::create_web_session(&state.pool, user.id, &token_hash, expires_at).await?;

    let is_prod = state.config.app_env == "production";
    let cookie = build_session_cookie(&raw_token, is_prod);

    let mut resp = ApiResponse::ok(
        WebAuthProfile {
            id: user.id,
            email: user.email.unwrap_or(email_lower),
            full_name: user.full_name,
            ref_code: user.ref_code,
            balance_vnd: wallet.balance_vnd,
            total_deposited_vnd: wallet.total_deposited_vnd,
        },
        "Đăng nhập thành công",
        ctx.request_id,
    )
    .into_response();

    if let Ok(val) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(SET_COOKIE, val);
    }
    Ok(resp)
}

/// Logout and immediately revoke server-side session token in PostgreSQL.
pub async fn web_logout(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    require_csrf_xhr(&headers)?;

    if let Some(raw_token) = extract_cookie(&headers, WEB_SESSION_COOKIE_NAME) {
        let token_hash = sha256_hex(&raw_token);
        let _ = user_repo::delete_web_session(&state.pool, &token_hash).await;
    }

    let is_prod = state.config.app_env == "production";
    let cookie = build_clear_session_cookie(is_prod);
    let mut resp = ApiResponse::ok(
        serde_json::json!({ "logged_out": true }),
        "Đã đăng xuất an toàn",
        ctx.request_id,
    )
    .into_response();

    if let Ok(val) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(SET_COOKIE, val);
    }
    Ok(resp)
}

#[derive(Debug, Serialize)]
pub struct WebMeRes {
    pub user: WebAuthProfile,
    pub referral_url: String,
    pub referral_summary: ReferralSummary,
}

/// Get authenticated web user's profile, live wallet balance & referral summary.
pub async fn web_me(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
) -> Result<ApiResponse<WebMeRes>, AppError> {
    let wallet = wallet_repo::get_by_user_id(&state.pool, user.id).await?;
    let referral_summary = referral_service::get_referral_summary(&state.pool, user.id).await?;
    let referral_url = format!("{}/?ref={}", state.config.public_base_url, user.ref_code);

    Ok(ApiResponse::ok(
        WebMeRes {
            user: WebAuthProfile {
                id: user.id,
                email: user.email.unwrap_or_default(),
                full_name: user.full_name,
                ref_code: user.ref_code,
                balance_vnd: wallet.balance_vnd,
                total_deposited_vnd: wallet.total_deposited_vnd,
            },
            referral_url,
            referral_summary,
        },
        "Lấy thông tin tài khoản thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Serialize)]
pub struct WebOrderView {
    pub id: Uuid,
    pub order_code: String,
    pub plan_name: String,
    pub plan_api_mode: String,
    pub price_paid_vnd: i64,
    pub gmail_masked: String,
    pub status: String,
    pub external_status: Option<String>,
    pub external_step: Option<String>,
    pub external_result: Option<String>,
    pub admin_note: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// List ONLY the authenticated user's own orders (zero decrypted credentials exposed).
pub async fn web_list_orders(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
) -> Result<ApiResponse<Vec<WebOrderView>>, AppError> {
    let orders = activation_repo::list_by_user(&state.pool, user.id, 30).await?;
    let plans = plan_repo::list_all_plans(&state.pool).await.unwrap_or_default();

    let views: Vec<WebOrderView> = orders
        .into_iter()
        .map(|o| {
            let plan = plans.iter().find(|p| p.id == o.plan_id);
            WebOrderView {
                id: o.id,
                order_code: o.order_code,
                plan_name: plan
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Gemini Pro".to_string()),
                plan_api_mode: plan
                    .map(|p| p.api_mode.clone())
                    .unwrap_or_else(|| "autopay".to_string()),
                price_paid_vnd: o.price_paid_vnd,
                gmail_masked: o.gmail_masked,
                status: o.status,
                external_status: o.external_status,
                external_step: o.external_step,
                external_result: o.external_result,
                admin_note: o.admin_note,
                created_at: o.created_at,
            }
        })
        .collect();

    Ok(ApiResponse::ok(
        views,
        "Lấy danh sách đơn hàng thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct WebCreateOrderReq {
    pub plan_id: Uuid,
    pub email: String,
    pub password: String,
    pub two_fa_secret: String,
    pub backup_codes: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct WebCreateOrderRes {
    pub order_id: Uuid,
    pub order_code: String,
    pub gmail_masked: String,
    pub price_paid_vnd: i64,
    pub remaining_balance_vnd: i64,
    pub status: String,
}

/// Submit an activation order from the public Website (same ACID tx + AES-256-GCM encryption).
pub async fn web_create_order(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
    Json(payload): Json<WebCreateOrderReq>,
) -> Result<ApiResponse<WebCreateOrderRes>, AppError> {
    let outcome = state
        .activation_service
        .submit_activation_order_for_user(
            user,
            payload.plan_id,
            &payload.email,
            &payload.password,
            &payload.two_fa_secret,
            payload.backup_codes.as_deref(),
        )
        .await?;

    let auto_push_enabled = settings_repo::get_settings(&state.pool)
        .await
        .map(|s| s.smm_auto_push && !s.smm_api_key.trim().is_empty())
        .unwrap_or(false);

    // Notify Admin via Telegram with exact copy-ready format
    if let Some(admin_id) = state.config.admin_telegram_id {
        let mode_line = if auto_push_enabled {
            format!(
                "\n🚀 Hàng đợi tự động đang BẬT — sẽ tự đẩy sang API ({})",
                outcome.plan.api_mode
            )
        } else {
            "\n📥 Trạng thái: Đang nằm trong hàng chờ (PENDING)".to_string()
        };
        let web_email = outcome.user.email.as_deref().unwrap_or("Web User");
        let admin_msg = format!(
            "🌐 <b>ĐƠN KÍCH HOẠT MỚI TỪ WEBSITE!</b>\n\
             • Mã đơn: <code>{}</code>\n\
             • Gói: <b>{}</b> ({} đ)\n\
             • Khách Web: <b>{}</b> (<code>{}</code>){}\n\n\
             📋 <b>Thông tin chuẩn (chạm để copy):</b>\n\
             <code>{}</code>",
            outcome.order.order_code,
            escape_html(&outcome.plan.name),
            format_vnd(outcome.order.price_paid_vnd),
            escape_html(&outcome.user.full_name),
            escape_html(web_email),
            mode_line,
            escape_html(&outcome.formatted_credentials),
        );
        let _ = state
            .bot
            .send_message(ChatId(admin_id), admin_msg)
            .parse_mode(ParseMode::Html)
            .await;
    }

    Ok(ApiResponse::created(
        WebCreateOrderRes {
            order_id: outcome.order.id,
            order_code: outcome.order.order_code,
            gmail_masked: outcome.order.gmail_masked,
            price_paid_vnd: outcome.order.price_paid_vnd,
            remaining_balance_vnd: outcome.remaining_balance_vnd,
            status: outcome.order.status,
        },
        "Đặt đơn kích hoạt thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct WebCreateDepositReq {
    pub amount_vnd: i64,
}

#[derive(Debug, Serialize)]
pub struct WebCreateDepositRes {
    pub order_code: i64,
    pub amount_vnd: i64,
    pub description: String,
    pub qr_image_url: String,
    pub account_number: Option<String>,
    pub account_name: Option<String>,
}

/// Create a VietQR top-up order for the authenticated web customer.
pub async fn web_create_deposit(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
    Json(payload): Json<WebCreateDepositReq>,
) -> Result<ApiResponse<WebCreateDepositRes>, AppError> {
    let created = state
        .payos_service
        .create_topup_link(user.id, payload.amount_vnd)
        .await?;

    let encoded_qr: String =
        url::form_urlencoded::byte_serialize(created.qr_code.as_bytes()).collect();
    let qr_image_url = format!(
        "https://api.qrserver.com/v1/create-qr-code/?size=360x360&margin=14&data={encoded_qr}"
    );

    Ok(ApiResponse::created(
        WebCreateDepositRes {
            order_code: created.deposit.order_code,
            amount_vnd: created.deposit.amount_vnd,
            description: created.deposit.description,
            qr_image_url,
            account_number: created.account_number,
            account_name: created.account_name,
        },
        "Đã tạo mã QR chuyển khoản thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Serialize)]
pub struct WebCheckDepositRes {
    pub status: String,
    pub amount_vnd: Option<i64>,
    pub new_balance_vnd: Option<i64>,
}

/// Check & confirm VietQR payment status (strictly verifying deposit belongs to authenticated user).
pub async fn web_check_deposit(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
    Path(order_code): Path<i64>,
) -> Result<ApiResponse<WebCheckDepositRes>, AppError> {
    // Ownership check (IDOR prevention)
    let owner: Option<(Uuid,)> =
        sqlx::query_as("SELECT user_id FROM payos_deposits WHERE order_code = $1")
            .bind(order_code)
            .fetch_optional(&state.pool)
            .await?;

    match owner {
        Some((uid,)) if uid == user.id => {}
        _ => {
            return Err(AppError::NotFound(
                "Không tìm thấy giao dịch nạp tiền này.".to_string(),
            ));
        }
    }

    match state
        .payos_service
        .check_and_confirm_payment(order_code)
        .await?
    {
        CheckPaymentStatus::PaidNow(outcome) => {
            if let Some(ref_reward) = outcome.referral_reward {
                if ref_reward.referrer_telegram_id > 0 {
                    let ref_msg = format!(
                        "🎉 <b>Bạn vừa nhận thưởng giới thiệu!</b>\n\n\
                         👤 <b>{}</b> vừa hoàn tất lần nạp tiền đầu tiên.\n\
                         💰 Đã cộng <b>+{} đ</b> vào ví của bạn!",
                        escape_html(&ref_reward.referred_user_name),
                        format_vnd(ref_reward.reward_vnd),
                    );
                    let _ = state
                        .bot
                        .send_message(ChatId(ref_reward.referrer_telegram_id), ref_msg)
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            }
            Ok(ApiResponse::ok(
                WebCheckDepositRes {
                    status: "PAID".to_string(),
                    amount_vnd: Some(outcome.amount_vnd),
                    new_balance_vnd: Some(outcome.new_balance_vnd),
                },
                "Xác nhận thanh toán thành công! Tiền đã được cộng vào ví.",
                ctx.request_id,
            ))
        }
        CheckPaymentStatus::AlreadyPaid { amount_vnd, .. } => {
            let bal = wallet_repo::get_by_user_id(&state.pool, user.id)
                .await
                .map(|w| w.balance_vnd)
                .ok();
            Ok(ApiResponse::ok(
                WebCheckDepositRes {
                    status: "PAID".to_string(),
                    amount_vnd: Some(amount_vnd),
                    new_balance_vnd: bal,
                },
                "Giao dịch này đã được cộng tiền vào ví trước đó.",
                ctx.request_id,
            ))
        }
        CheckPaymentStatus::StillPending { .. } => Ok(ApiResponse::ok(
            WebCheckDepositRes {
                status: "PENDING".to_string(),
                amount_vnd: None,
                new_balance_vnd: None,
            },
            "Hệ thống chưa nhận được tiền. Nếu bạn vừa chuyển khoản, hãy đợi 10–30 giây rồi bấm kiểm tra lại.",
            ctx.request_id,
        )),
        CheckPaymentStatus::Cancelled { .. } => Ok(ApiResponse::ok(
            WebCheckDepositRes {
                status: "CANCELLED".to_string(),
                amount_vnd: None,
                new_balance_vnd: None,
            },
            "Giao dịch đã bị hủy hoặc hết hạn.",
            ctx.request_id,
        )),
    }
}

/// Cancel a pending VietQR deposit (strictly scoped to authenticated user).
pub async fn web_cancel_deposit(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
    Path(order_code): Path<i64>,
) -> Result<ApiResponse<serde_json::Value>, AppError> {
    let cancelled = state
        .payos_service
        .cancel_payment(order_code, user.id)
        .await?;
    Ok(ApiResponse::ok(
        serde_json::json!({ "cancelled": cancelled }),
        if cancelled {
            "Đã hủy yêu cầu nạp tiền."
        } else {
            "Giao dịch đã hoàn tất hoặc đã hủy trước đó."
        },
        ctx.request_id,
    ))
}

/// List authenticated web user's immutable wallet ledger transactions.
pub async fn web_wallet_transactions(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
) -> Result<ApiResponse<Vec<WalletTransaction>>, AppError> {
    let wallet = wallet_repo::get_by_user_id(&state.pool, user.id).await?;
    let txs = wallet_repo::list_recent_transactions(&state.pool, wallet.id, 30).await?;
    Ok(ApiResponse::ok(
        txs,
        "Lấy lịch sử ví thành công",
        ctx.request_id,
    ))
}

#[derive(Debug, Deserialize)]
pub struct WebBuyPartnerReq {
    pub product_id: Uuid,
    pub quantity: i32,
}

#[derive(Debug, Serialize)]
pub struct WebPartnerOrderView {
    pub id: Uuid,
    pub order_code: String,
    pub product_name: String,
    pub quantity: i32,
    pub total_sell_price_vnd: i64,
    pub status: String,
    pub items: Vec<String>,
    pub error_note: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct WebBuyPartnerRes {
    pub order: WebPartnerOrderView,
    pub remaining_balance_vnd: i64,
}

/// Purchase a synced Partner Shop product (account/key) directly from Website with instant delivery.
pub async fn web_buy_partner_product(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
    Json(payload): Json<WebBuyPartnerReq>,
) -> Result<ApiResponse<WebBuyPartnerRes>, AppError> {
    let outcome = partner_shop_service::purchase_partner_product(
        &state,
        user,
        payload.product_id,
        payload.quantity,
    )
    .await?;

    Ok(ApiResponse::created(
        WebBuyPartnerRes {
            order: WebPartnerOrderView {
                id: outcome.order.id,
                order_code: outcome.order.order_code,
                product_name: outcome.order.product_name,
                quantity: outcome.order.quantity,
                total_sell_price_vnd: outcome.order.total_sell_price_vnd,
                status: outcome.order.status,
                items: outcome.items,
                error_note: outcome.order.error_note,
                created_at: outcome.order.created_at,
            },
            remaining_balance_vnd: outcome.remaining_balance_vnd,
        },
        "Mua sản phẩm thành công! Thông tin tài khoản/key đã được giao ngay.",
        ctx.request_id,
    ))
}

/// List ONLY the authenticated user's own Partner Shop orders with decrypted delivered items.
pub async fn web_list_partner_orders(
    State(state): State<SharedState>,
    Extension(ctx): Extension<RequestContext>,
    Extension(WebAuthUser(user)): Extension<WebAuthUser>,
) -> Result<ApiResponse<Vec<WebPartnerOrderView>>, AppError> {
    let orders = partner_repo::list_orders_by_user(&state.pool, user.id, 30).await?;
    let views: Vec<WebPartnerOrderView> = orders
        .into_iter()
        .map(|o| {
            let items = partner_shop_service::decrypt_order_items(&state, &o);
            WebPartnerOrderView {
                id: o.id,
                order_code: o.order_code,
                product_name: o.product_name,
                quantity: o.quantity,
                total_sell_price_vnd: o.total_sell_price_vnd,
                status: o.status,
                items,
                error_note: o.error_note,
                created_at: o.created_at,
            }
        })
        .collect();

    Ok(ApiResponse::ok(
        views,
        "Lấy lịch sử đơn tài khoản/key thành công",
        ctx.request_id,
    ))
}
