use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};
use url::Url;
use uuid::Uuid;

use crate::{
    bot::messages::escape_html,
    domain::wallet::format_vnd,
    error::AppError,
    repo::{activation_repo, settings_repo},
    state::SharedState,
};

#[derive(Debug, Serialize)]
struct PixelSubmitBody<'a> {
    gmail: &'a str,
    password: &'a str,
    totp: &'a str,
    mode: &'a str,
}

#[derive(Debug, Deserialize)]
struct PixelSubmitResp {
    ok: Option<bool>,
    message: Option<String>,
    error: Option<String>,
    results: Option<Vec<PixelSubmitItem>>,
}

#[derive(Debug, Deserialize)]
struct PixelSubmitItem {
    ok: Option<bool>,
    job_id: Option<i64>,
    position: Option<i64>,
    error: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubmittedExternalJob {
    pub job_id: i64,
    pub position: Option<i64>,
    pub mode: String,
}

/// Fetch upstream partner account info (`GET /api/v2/info`) to verify API key & show balance in /admin.
pub async fn fetch_partner_info(
    base_url: &str,
    api_key: &str,
) -> Result<serde_json::Value, AppError> {
    if api_key.trim().is_empty() {
        return Err(AppError::Validation(
            "Chưa cấu hình API Key đối tác (smm_api_key).".to_string(),
        ));
    }
    let url = format!("{}/api/v2/info", base_url.trim_end_matches('/'));
    let resp = Client::new()
        .get(&url)
        .header("X-API-Key", api_key.trim())
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Không thể kết nối API đối tác: {e}")))?;

    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Phản hồi API đối tác không hợp lệ: {e}")))?;

    Ok(val)
}

/// Submit a single Gmail job to `POST /api/v2/pixel/submit`.
pub async fn submit_pixel_job(
    base_url: &str,
    api_key: &str,
    gmail: &str,
    password: &str,
    totp_compact_upper: &str,
    mode: &str,
) -> Result<SubmittedExternalJob, AppError> {
    if api_key.trim().is_empty() {
        return Err(AppError::Validation(
            "Chưa cấu hình API Key đối tác trong trang quản trị.".to_string(),
        ));
    }
    let url = format!("{}/api/v2/pixel/submit", base_url.trim_end_matches('/'));
    let body = PixelSubmitBody {
        gmail,
        password,
        totp: totp_compact_upper,
        mode,
    };

    let resp = Client::new()
        .post(&url)
        .header("X-API-Key", api_key.trim())
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Lỗi gửi đơn sang API đối tác: {e}")))?;

    let parsed: PixelSubmitResp = resp.json().await.map_err(|e| {
        AppError::PaymentGateway(format!("Không đọc được phản hồi submit từ API đối tác: {e}"))
    })?;

    if !parsed.ok.unwrap_or(false) {
        let msg = parsed
            .message
            .or(parsed.error)
            .unwrap_or_else(|| "API đối tác từ chối đơn".to_string());
        return Err(AppError::PaymentGateway(msg));
    }

    let first = parsed
        .results
        .and_then(|mut v| if v.is_empty() { None } else { Some(v.remove(0)) })
        .ok_or_else(|| {
            AppError::PaymentGateway("API đối tác không trả về kết quả job_id".to_string())
        })?;

    if !first.ok.unwrap_or(false) {
        let msg = first
            .message
            .or(first.error)
            .unwrap_or_else(|| "Tài khoản không hợp lệ trên API đối tác".to_string());
        return Err(AppError::PaymentGateway(msg));
    }

    let job_id = first.job_id.ok_or_else(|| {
        AppError::PaymentGateway("Thiếu job_id trong phản hồi API đối tác".to_string())
    })?;

    Ok(SubmittedExternalJob {
        job_id,
        position: first.position,
        mode: mode.to_string(),
    })
}

/// Spawn a background Tokio task to poll `GET /api/v2/pixel/status/{job_id}` and edit
/// the customer's Telegram progress message in real time!
#[allow(clippy::too_many_arguments)]
pub fn spawn_live_job_tracker(
    state: SharedState,
    order_id: Uuid,
    order_code: String,
    user_telegram_id: i64,
    gmail: String,
    job_id: i64,
    initial_position: Option<i64>,
    mode: String,
    base_url: String,
    api_key: String,
) {
    tokio::spawn(async move {
        let started = Instant::now();
        let http = Client::new();

        // 1. Send initial live status message to customer
        let init_text = render_progress_card(
            &gmail,
            &mode,
            "queued",
            initial_position,
            None,
            None,
            None,
            0,
        );

        let sent_msg = match state
            .bot
            .send_message(ChatId(user_telegram_id), &init_text)
            .parse_mode(ParseMode::Html)
            .await
        {
            Ok(m) => Some(m),
            Err(err) => {
                tracing::error!(error = ?err, "Failed to send initial live progress message");
                None
            }
        };

        let mut last_rendered = init_text;
        let status_url = format!(
            "{}/api/v2/pixel/status/{}",
            base_url.trim_end_matches('/'),
            job_id
        );

        // Poll up to 180 times (~12 minutes at 4s interval)
        for _ in 0..180 {
            tokio::time::sleep(Duration::from_secs(4)).await;
            let elapsed_secs = started.elapsed().as_secs();

            let resp = match http
                .get(&status_url)
                .header("X-API-Key", api_key.trim())
                .timeout(Duration::from_secs(15))
                .send()
                .await
            {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!(error = ?err, job_id, "Temporary error polling Pixel job status");
                    continue;
                }
            };

            let Ok(val) = resp.json::<serde_json::Value>().await else {
                continue;
            };

            let status = val
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("running")
                .to_lowercase();
            let position = val.get("position").and_then(|v| v.as_i64());
            let current_step = val
                .get("current_step")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let error_msg = val
                .get("error_msg")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);

            // Extract offer/result link (primary field from api.smm.io.vn is `extract_link`)
            let offer_link = extract_link_from_response(&val);

            let _ = activation_repo::update_external_progress(
                &state.pool,
                order_id,
                &status,
                current_step.as_deref(),
                offer_link.as_deref().or(error_msg.as_deref()),
            )
            .await;

            let rendered = render_progress_card(
                &gmail,
                &mode,
                &status,
                position,
                current_step.as_deref(),
                offer_link.as_deref(),
                error_msg.as_deref(),
                elapsed_secs,
            );

            if rendered != last_rendered {
                if let Some(ref msg) = sent_msg {
                    let mut req = state
                        .bot
                        .edit_message_text(msg.chat.id, msg.id, &rendered)
                        .parse_mode(ParseMode::Html);
                    if status == "success" {
                        if let Some(ref link) = offer_link {
                            if let Ok(parsed_url) = Url::parse(link) {
                                let kb = InlineKeyboardMarkup::new(vec![vec![
                                    InlineKeyboardButton::url("🔗 Mở link ưu đãi ngay", parsed_url),
                                ]]);
                                req = req.reply_markup(kb);
                            }
                        }
                    }
                    let _ = req.await;
                }
                last_rendered = rendered;
            }

            if status == "success" {
                let note = offer_link
                    .as_deref()
                    .map(|l| format!("Link ưu đãi: {l}"))
                    .unwrap_or_else(|| format!("Kích hoạt tự động thành công (Job #{job_id})"));
                let _ = state
                    .activation_service
                    .complete_order(order_id, Some(&note))
                    .await;
                return;
            }

            if status == "failed" || status == "cancelled" {
                let reason = error_msg
                    .unwrap_or_else(|| "Hệ thống tự động báo xử lý thất bại".to_string());
                let auto_refund = settings_repo::get_settings(&state.pool)
                    .await
                    .map(|s| s.smm_auto_refund_on_fail)
                    .unwrap_or(true);

                if auto_refund {
                    if let Ok(refund_out) = state
                        .activation_service
                        .refund_failed_order(order_id, &reason)
                        .await
                    {
                        let new_bal = refund_out.refunded_balance_vnd.unwrap_or(0);
                        let refund_msg = format!(
                            "↩️ <b>Đã tự động hoàn tiền 100% đơn lỗi!</b>\n\n\
                             • Mã đơn: <code>{}</code>\n\
                             • Hoàn lại ví: <b>+{} đ</b>\n\
                             • Số dư hiện tại: <b>{} đ</b>\n\
                             • Lý do: <i>{}</i>",
                            order_code,
                            format_vnd(refund_out.order.price_paid_vnd),
                            format_vnd(new_bal),
                            escape_html(&reason),
                        );
                        let _ = state
                            .bot
                            .send_message(ChatId(user_telegram_id), refund_msg)
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                }
                return;
            }
        }
    });
}

fn extract_link_from_response(val: &serde_json::Value) -> Option<String> {
    let known_keys = [
        "extract_link",
        "offer_url",
        "offer_link",
        "link",
        "result_url",
        "result",
        "url",
    ];
    for k in known_keys {
        if let Some(s) = val.get(k).and_then(|v| v.as_str()).map(str::trim) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    // Bulletproof fallback: scan any string field in the response for an https:// URL
    if let Some(obj) = val.as_object() {
        for v in obj.values() {
            if let Some(s) = v.as_str().map(str::trim) {
                if s.starts_with("https://") || s.starts_with("http://") {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

fn step_index_from_name(step: Option<&str>) -> usize {
    let Some(s) = step else {
        return 0;
    };
    let lower = s.to_lowercase();
    if lower.contains("claim")
        || lower.contains("autopay")
        || lower.contains("payadd")
        || lower.contains("extract")
        || lower.contains("finish")
        || lower.contains("done")
    {
        6
    } else if lower.contains("offer") || lower.contains("pixel") || lower.contains("eligib") {
        5
    } else if lower.contains("2fa") || lower.contains("totp") || lower.contains("otp") || lower.contains("verify") {
        4
    } else if lower.contains("pass") || lower.contains("login") {
        3
    } else if lower.contains("spam") || lower.contains("captcha") {
        2
    } else {
        1
    }
}

#[allow(clippy::too_many_arguments)]
fn render_progress_card(
    gmail: &str,
    mode: &str,
    status: &str,
    position: Option<i64>,
    current_step: Option<&str>,
    offer_link: Option<&str>,
    error_msg: Option<&str>,
    elapsed_secs: u64,
) -> String {
    let title = if mode == "extract" {
        "Trích xuất ưu đãi tự động"
    } else {
        "Kích hoạt Gemini Pro tự động"
    };

    let steps = [
        "1. Email",
        "2. Kiểm tra spam",
        "3. Mật khẩu",
        "4. Xác thực 2 bước",
        "5. Kiểm tra ưu đãi",
        "6. Nhận ưu đãi",
    ];

    let active_idx = match status {
        "success" => 7,
        "queued" => 0,
        _ => step_index_from_name(current_step).max(1),
    };

    let mut step_lines = String::new();
    for (i, label) in steps.iter().enumerate() {
        let num = i + 1;
        let icon = if status == "success" || num < active_idx {
            "✅"
        } else if (status == "failed" || status == "cancelled") && num == active_idx {
            "❌"
        } else if num == active_idx {
            "🔄"
        } else {
            "⬜"
        };
        step_lines.push_str(&format!("{icon} {label}\n"));
    }

    let mins = elapsed_secs / 60;
    let secs = elapsed_secs % 60;
    let time_str = format!("{mins:02}:{secs:02}");

    let footer = match status {
        "success" => {
            if let Some(link) = offer_link {
                format!(
                    "🎉 <b>Hoàn tất thành công!</b>\n🔗 Link ưu đãi:\n<code>{}</code>",
                    escape_html(link)
                )
            } else {
                "🎉 <b>Kích hoạt Gemini Pro thành công!</b>".to_string()
            }
        }
        "failed" | "cancelled" => {
            let err = error_msg.unwrap_or("Tài khoản không đủ điều kiện hoặc sai thông tin.");
            format!("❌ <b>Thất bại:</b> {}", escape_html(err))
        }
        "queued" => {
            let pos = position
                .map(|p| format!(" (vị trí #{p})"))
                .unwrap_or_default();
            format!("⏳ Đang xếp hàng xử lý tự động{pos}...")
        }
        _ => {
            let step_txt = current_step.unwrap_or("đang xử lý");
            format!("⚙️ Đang xử lý ({})...", escape_html(step_txt))
        }
    };

    format!(
        "<b>{title}</b>\n\
         📧 <code>{}</code>\n\
         ━━━━━━━━━━━━━━━━━━━━\n\
         {}\
         ━━━━━━━━━━━━━━━━━━━━\n\
         {}\n\
         ⏱️ Thời gian: <code>{}</code>",
        escape_html(gmail),
        step_lines,
        footer,
        time_str,
    )
}
