use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, InputFile, ParseMode, WebAppInfo},
    utils::command::BotCommands,
};
use url::Url;

use crate::{
    bot::{keyboards, messages},
    domain::wallet::format_vnd,
    repo::{activation_repo, partner_repo, plan_repo, settings_repo, user_repo, wallet_repo},
    services::{partner_shop_service, referral_service},
    state::SharedState,
};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Danh sách lệnh hỗ trợ:")]
pub enum Command {
    #[command(description = "▶️ Xem menu chính")]
    Start(String),
    #[command(description = "🤖 Kích hoạt Gemini Pro chính chủ")]
    Pixel,
    #[command(description = "🛍️ Cửa hàng tài khoản & Key giao tự động")]
    Shop,
    #[command(description = "💳 Nạp tiền tự động qua ngân hàng (VietQR)")]
    Payadd,
    #[command(description = "🎁 Mời bạn bè & xem thống kê tiền thưởng")]
    Ref,
    #[command(description = "📘 Hướng dẫn đóng HSTT & lấy mã 2FA")]
    Guide,
}

pub async fn handle_message(bot: Bot, msg: Message, state: SharedState) -> ResponseResult<()> {
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let telegram_id = from.id.0 as i64;
    let text = msg.text().unwrap_or("").trim();

    if let Ok(cmd) = Command::parse(text, "") {
        state.awaiting_deposit_amount.write().await.remove(&telegram_id);
        match cmd {
            Command::Start(start_arg) => {
                handle_start(&bot, &msg, from, start_arg.trim(), &state).await?;
            }
            Command::Pixel => {
                handle_show_plans(&bot, msg.chat.id, &state).await?;
            }
            Command::Shop => {
                handle_show_partner_shop(&bot, msg.chat.id, &state).await?;
            }
            Command::Payadd => {
                handle_payadd(&bot, msg.chat.id, telegram_id, &state).await?;
            }
            Command::Ref => {
                handle_referral_menu(&bot, msg.chat.id, telegram_id, &state).await?;
            }
            Command::Guide => {
                handle_guide(&bot, msg.chat.id, &state).await?;
            }
        }
        return Ok(());
    }

    // Handle Reply Keyboard button taps
    match text {
        keyboards::BTN_GEMINI_PRO => {
            state.awaiting_deposit_amount.write().await.remove(&telegram_id);
            handle_show_plans(&bot, msg.chat.id, &state).await?;
            return Ok(());
        }
        keyboards::BTN_PARTNER_SHOP => {
            state.awaiting_deposit_amount.write().await.remove(&telegram_id);
            handle_show_partner_shop(&bot, msg.chat.id, &state).await?;
            return Ok(());
        }
        keyboards::BTN_WALLET => {
            state.awaiting_deposit_amount.write().await.remove(&telegram_id);
            handle_payadd(&bot, msg.chat.id, telegram_id, &state).await?;
            return Ok(());
        }
        keyboards::BTN_ORDERS => {
            state.awaiting_deposit_amount.write().await.remove(&telegram_id);
            handle_my_orders(&bot, msg.chat.id, telegram_id, &state).await?;
            return Ok(());
        }
        keyboards::BTN_GUIDE => {
            state.awaiting_deposit_amount.write().await.remove(&telegram_id);
            handle_guide(&bot, msg.chat.id, &state).await?;
            return Ok(());
        }
        keyboards::BTN_REFERRAL => {
            state.awaiting_deposit_amount.write().await.remove(&telegram_id);
            handle_referral_menu(&bot, msg.chat.id, telegram_id, &state).await?;
            return Ok(());
        }
        _ => {}
    }

    // Check if user is entering a custom deposit amount
    let is_awaiting = state
        .awaiting_deposit_amount
        .read()
        .await
        .contains(&telegram_id);

    if is_awaiting {
        let digits_only: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
        let Ok(amount_vnd) = digits_only.parse::<i64>() else {
            bot.send_message(
                msg.chat.id,
                "⚠️ Số tiền không hợp lệ. Vui lòng chỉ nhập số (ví dụ: <code>50000</code> hoặc <code>100000</code>):",
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboards::back_to_home_keyboard())
            .await?;
            return Ok(());
        };

        let min_deposit = settings_repo::get_settings(&state.pool)
            .await
            .map(|s| s.min_deposit_vnd)
            .unwrap_or(10_000);

        if amount_vnd < min_deposit {
            bot.send_message(
                msg.chat.id,
                format!(
                    "⚠️ Số tiền nạp tối thiểu mỗi lần là <b>{} đ</b>. Vui lòng nhập số tiền lớn hơn hoặc bằng {} đ:",
                    format_vnd(min_deposit),
                    format_vnd(min_deposit)
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboards::back_to_home_keyboard())
            .await?;
            return Ok(());
        }

        state.awaiting_deposit_amount.write().await.remove(&telegram_id);
        send_vietqr_deposit_photo(&bot, msg.chat.id, telegram_id, amount_vnd, &state).await?;
    }

    Ok(())
}

/// Helper: Create VietQR payment and send the QR image directly inside Telegram chat.
pub async fn send_vietqr_deposit_photo(
    bot: &Bot,
    chat_id: ChatId,
    telegram_id: i64,
    amount_vnd: i64,
    state: &SharedState,
) -> ResponseResult<()> {
    let Ok(Some(user)) = user_repo::find_by_telegram_id(&state.pool, telegram_id).await else {
        bot.send_message(chat_id, "⚠️ Vui lòng bấm /start trước để khởi tạo ví.")
            .await?;
        return Ok(());
    };

    match state.payos_service.create_topup_link(user.id, amount_vnd).await {
        Ok(created) => {
            let caption = messages::vietqr_photo_caption(
                created.deposit.order_code,
                created.deposit.amount_vnd,
                &created.deposit.description,
                created.account_number.as_deref(),
                created.account_name.as_deref(),
            );
            let encoded_qr: String =
                url::form_urlencoded::byte_serialize(created.qr_code.as_bytes()).collect();
            let qr_image_url = format!(
                "https://api.qrserver.com/v1/create-qr-code/?size=420x420&margin=16&data={encoded_qr}"
            );

            let kb = keyboards::vietqr_payment_actions_keyboard(created.deposit.order_code);
            if let Ok(parsed_url) = Url::parse(&qr_image_url) {
                let photo_res = bot
                    .send_photo(chat_id, InputFile::url(parsed_url))
                    .caption(caption.clone())
                    .parse_mode(ParseMode::Html)
                    .reply_markup(kb.clone())
                    .await;

                if photo_res.is_ok() {
                    return Ok(());
                }
            }

            // Fallback to text if photo send fails
            bot.send_message(chat_id, caption)
                .parse_mode(ParseMode::Html)
                .reply_markup(kb)
                .await?;
        }
        Err(err) => {
            tracing::error!(error = ?err, "Failed to create VietQR topup link");
            bot.send_message(chat_id, format!("⚠️ Không thể tạo mã nạp tiền lúc này: {err}"))
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
        }
    }

    Ok(())
}

/// Reusable home menu screen (used by `/start` and `back:home`).
pub async fn send_home_menu(
    bot: &Bot,
    chat_id: ChatId,
    telegram_id: i64,
    fallback_name: &str,
    state: &SharedState,
) -> ResponseResult<()> {
    let settings = settings_repo::get_settings(&state.pool).await.ok();
    let reward_vnd = settings
        .as_ref()
        .map(|s| s.referral_reward_vnd)
        .unwrap_or(state.config.referral_reward_vnd);
    let referred_bonus_vnd = settings
        .as_ref()
        .map(|s| s.referred_bonus_vnd)
        .unwrap_or(2_000);
    let min_first_deposit = settings
        .as_ref()
        .map(|s| s.referral_min_first_deposit_vnd)
        .unwrap_or(10_000);
    let support_handle = settings
        .as_ref()
        .map(|s| s.support_handle.as_str())
        .unwrap_or(state.config.support_handle.as_str());

    let (full_name, ref_code, balance) =
        match user_repo::find_by_telegram_id(&state.pool, telegram_id).await {
            Ok(Some(user)) => {
                let bal = wallet_repo::get_by_user_id(&state.pool, user.id)
                    .await
                    .map(|w| w.balance_vnd)
                    .unwrap_or(0);
                (user.full_name, user.ref_code, bal)
            }
            _ => (fallback_name.to_string(), "START".to_string(), 0),
        };

    let bot_username = bot
        .get_me()
        .await
        .ok()
        .and_then(|me| me.username.clone())
        .unwrap_or_else(|| "your_bot".to_string());
    let ref_link = format!("https://t.me/{}?start=ref_{}", bot_username, ref_code);

    let text = messages::welcome_message(
        &full_name,
        balance,
        &ref_link,
        reward_vnd,
        referred_bonus_vnd,
        min_first_deposit,
        support_handle,
    );

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboards::main_reply_keyboard())
        .await?;

    Ok(())
}

async fn handle_start(
    bot: &Bot,
    msg: &Message,
    from: &teloxide::types::User,
    start_arg: &str,
    state: &SharedState,
) -> ResponseResult<()> {
    let telegram_id = from.id.0 as i64;
    let full_name = match &from.last_name {
        Some(last) if !last.is_empty() => format!("{} {}", from.first_name, last),
        _ => from.first_name.clone(),
    };

    let ref_code_opt = start_arg
        .strip_prefix("ref_")
        .or(if !start_arg.is_empty() {
            Some(start_arg)
        } else {
            None
        });

    let mut referrer = None;
    if let Some(code) = ref_code_opt {
        if let Ok(Some(found)) = user_repo::find_by_ref_code(&state.pool, code).await {
            if found.telegram_id != telegram_id {
                referrer = Some(found);
            }
        }
    }

    let Ok(mut tx) = state.pool.begin().await else {
        bot.send_message(msg.chat.id, "⚠️ Hệ thống đang bận, vui lòng thử lại sau.")
            .await?;
        return Ok(());
    };

    let settings = settings_repo::get_settings(&state.pool).await.ok();
    let reward_vnd = settings
        .as_ref()
        .map(|s| s.referral_reward_vnd)
        .unwrap_or(state.config.referral_reward_vnd);
    let referred_bonus_vnd = settings
        .as_ref()
        .map(|s| s.referred_bonus_vnd)
        .unwrap_or(2_000);
    let min_first_deposit = settings
        .as_ref()
        .map(|s| s.referral_min_first_deposit_vnd)
        .unwrap_or(10_000);

    let ensured = user_repo::ensure_user_with_wallet(
        &mut tx,
        telegram_id,
        from.username.as_deref(),
        &full_name,
        referrer.as_ref().map(|r| r.id),
        reward_vnd,
        referred_bonus_vnd,
    )
    .await;

    let (user, is_new) = match ensured {
        Ok(pair) => {
            if tx.commit().await.is_err() {
                bot.send_message(msg.chat.id, "⚠️ Lỗi lưu dữ liệu, vui lòng thử lại.")
                    .await?;
                return Ok(());
            }
            pair
        }
        Err(err) => {
            tracing::error!(error = ?err, "Failed to ensure user on /start");
            bot.send_message(msg.chat.id, "⚠️ Không thể khởi tạo tài khoản, vui lòng thử lại.")
                .await?;
            return Ok(());
        }
    };

    if is_new {
        if let Some(ref_user) = referrer {
            let notice = messages::referral_joined_notification(
                &user.full_name,
                reward_vnd,
                min_first_deposit,
            );
            let _ = bot
                .send_message(ChatId(ref_user.telegram_id), notice)
                .parse_mode(ParseMode::Html)
                .await;
        }
    }

    send_home_menu(bot, msg.chat.id, telegram_id, &full_name, state).await
}

pub async fn handle_show_plans(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let plans = match plan_repo::list_active_plans(&state.pool).await {
        Ok(p) => p,
        Err(err) => {
            tracing::error!(error = ?err, "Failed to list plans");
            bot.send_message(chat_id, "⚠️ Không thể tải danh sách gói Gemini Pro.")
                .await?;
            return Ok(());
        }
    };

    if plans.is_empty() {
        bot.send_message(chat_id, "Hiện chưa có gói kích hoạt Gemini Pro nào đang mở bán.")
            .reply_markup(keyboards::back_to_home_keyboard())
            .await?;
        return Ok(());
    }

    if plans.len() == 1 {
        let plan = &plans[0];
        let pid = plan.id.to_string();
        let guide_url = state.config.guide_url(Some(&pid));
        let support_handle = settings_repo::get_settings(&state.pool)
            .await
            .map(|s| s.support_handle)
            .unwrap_or_else(|_| state.config.support_handle.clone());
        let card = messages::gemini_plan_card(plan, &guide_url, &support_handle);
        bot.send_message(chat_id, card)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboards::plan_detail_keyboard(&pid, &guide_url))
            .await?;
    } else {
        bot.send_message(
            chat_id,
            "🤖 <b>Chọn gói kích hoạt Gemini Pro chính chủ (Your Mail) bên dưới:</b>",
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboards::plan_selector_keyboard(&plans))
        .await?;
    }

    Ok(())
}

pub async fn handle_show_partner_shop(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let products = match partner_repo::list_active_products(&state.pool).await {
        Ok(p) => p,
        Err(err) => {
            tracing::error!(error = ?err, "Failed to list partner shop products");
            bot.send_message(chat_id, "⚠️ Không thể tải danh sách sản phẩm lúc này.")
                .await?;
            return Ok(());
        }
    };

    if products.is_empty() {
        bot.send_message(
            chat_id,
            "🛍️ Hiện cửa hàng tài khoản & Key đang cập nhật kho hàng mới. Vui lòng quay lại sau nhé!",
        )
        .reply_markup(keyboards::back_to_home_keyboard())
        .await?;
        return Ok(());
    }

    bot.send_message(
        chat_id,
        "🛍️ <b>CỬA HÀNG TÀI KHOẢN &amp; KEY GIAO TỰ ĐỘNG 24/7</b>\n\
         👇 Chọn sản phẩm bên dưới để xem chi tiết và mua ngay:",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboards::partner_shop_list_keyboard(&products))
    .await?;

    Ok(())
}

pub async fn handle_payadd(
    bot: &Bot,
    chat_id: ChatId,
    telegram_id: i64,
    state: &SharedState,
) -> ResponseResult<()> {
    let balance = match user_repo::find_by_telegram_id(&state.pool, telegram_id).await {
        Ok(Some(user)) => wallet_repo::get_by_user_id(&state.pool, user.id)
            .await
            .map(|w| w.balance_vnd)
            .unwrap_or(0),
        _ => 0,
    };

    let settings = settings_repo::get_settings(&state.pool).await.ok();
    let min_deposit = settings.as_ref().map(|s| s.min_deposit_vnd).unwrap_or(10_000);
    let quick_amounts = settings
        .as_ref()
        .map(|s| s.parsed_quick_amounts())
        .unwrap_or_else(|| vec![40_000, 80_000, 150_000, 300_000]);

    let msg = format!(
        "💰 <b>Ví của bạn</b>\n\
         • Số dư khả dụng: <b>{} đ</b>\n\
         • Nạp tối thiểu mỗi lần: <b>{} đ</b>\n\n\
         💳 <b>Nạp tiền tự động qua chuyển khoản ngân hàng (VietQR 24/7)</b>\n\
         Chọn mệnh giá nạp nhanh hoặc bấm <b>✏️ Nhập số tiền khác</b> bên dưới:",
        format_vnd(balance),
        format_vnd(min_deposit),
    );

    bot.send_message(chat_id, msg)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboards::payos_amount_keyboard(None, min_deposit, &quick_amounts))
        .await?;

    Ok(())
}

async fn handle_my_orders(
    bot: &Bot,
    chat_id: ChatId,
    telegram_id: i64,
    state: &SharedState,
) -> ResponseResult<()> {
    let Ok(Some(user)) = user_repo::find_by_telegram_id(&state.pool, telegram_id).await else {
        bot.send_message(chat_id, "Vui lòng gõ /start trước để khởi tạo tài khoản.")
            .await?;
        return Ok(());
    };

    let orders = activation_repo::list_by_user(&state.pool, user.id, 10)
        .await
        .unwrap_or_default();
    let partner_orders = partner_repo::list_orders_by_user(&state.pool, user.id, 5)
        .await
        .unwrap_or_default();

    if orders.is_empty() && partner_orders.is_empty() {
        bot.send_message(
            chat_id,
            "📦 Bạn chưa có đơn hàng nào. Gõ /pixel hoặc /shop để mua hàng ngay!",
        )
        .reply_markup(keyboards::back_to_home_keyboard())
        .await?;
        return Ok(());
    }

    let mut lines = Vec::new();
    if !orders.is_empty() {
        lines.push("📦 <b>Đơn kích hoạt Gemini Pro gần nhất:</b>".to_string());
        for o in orders {
            let icon = match o.status.as_str() {
                "COMPLETED" => "✅",
                "REFUNDED" => "↩️",
                "PROCESSING" => "⚙️",
                _ => "⏳",
            };
            let mut line = format!(
                "{} <code>{}</code> | {} | <b>{} đ</b> | <i>{}</i>",
                icon,
                o.order_code,
                messages::escape_html(&o.gmail_masked),
                format_vnd(o.price_paid_vnd),
                o.status
            );
            if let Some(ref res) = o.external_result {
                if res.starts_with("http://") || res.starts_with("https://") {
                    line.push_str(&format!("\n   🔗 Link ưu đãi: <code>{}</code>", messages::escape_html(res)));
                }
            }
            lines.push(line);
        }
    }

    if !partner_orders.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("🛍️ <b>Đơn tài khoản &amp; Key gần nhất:</b>".to_string());
        for po in partner_orders {
            let icon = match po.status.as_str() {
                "COMPLETED" => "✅",
                "REFUNDED" => "↩️",
                _ => "⚙️",
            };
            let mut line = format!(
                "{} <code>{}</code> | {} x{} | <b>{} đ</b> | <i>{}</i>",
                icon,
                po.order_code,
                messages::escape_html(&po.product_name),
                po.quantity,
                format_vnd(po.total_sell_price_vnd),
                po.status
            );
            if po.status == "COMPLETED" {
                let items = partner_shop_service::decrypt_order_items(state, &po);
                for item in items.iter().take(3) {
                    line.push_str(&format!("\n   🔑 <code>{}</code>", messages::escape_html(item)));
                }
            }
            lines.push(line);
        }
    }

    bot.send_message(chat_id, lines.join("\n"))
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboards::back_to_home_keyboard())
        .await?;

    Ok(())
}

async fn handle_guide(bot: &Bot, chat_id: ChatId, state: &SharedState) -> ResponseResult<()> {
    let guide_url = state.config.guide_url(None);
    let mut rows = Vec::new();
    if let Ok(url) = Url::parse(&guide_url) {
        if url.scheme() == "https" {
            rows.push(vec![InlineKeyboardButton::web_app(
                "📘 Mở trang hướng dẫn 6 bước (đóng HSTT & 2FA)",
                WebAppInfo { url },
            )]);
        } else {
            rows.push(vec![InlineKeyboardButton::url(
                "📘 Mở trang hướng dẫn 6 bước (đóng HSTT & 2FA)",
                url,
            )]);
        }
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại menu chính",
        "back:home",
    )]);

    let text = "📘 <b>Hướng dẫn 6 bước chuẩn bị Gmail trước khi kích hoạt Gemini Pro</b>\n\n\
        <b>Phần 1 — Dọn sạch hồ sơ thanh toán Google (HSTT):</b>\n\
        1️⃣ Mở cài đặt Google Payments &amp; chọn đóng hồ sơ cũ\n\
        2️⃣ Chọn lý do &amp; xác nhận đóng hồ sơ\n\
        3️⃣ Tải lại trang kiểm tra dọn sạch toàn bộ hồ sơ\n\n\
        <b>Phần 2 — Bật bảo mật 2 lớp (2FA Authenticator):</b>\n\
        4️⃣ Mở cài đặt Authenticator Google\n\
        5️⃣ Bấm \"Không thể quét được mã?\" lấy chuỗi khóa 2FA 32 ký tự\n\
        6️⃣ Lấy mã OTP 6 số (có sẵn công cụ ngay trên trang) &amp; hoàn tất bật 2FA\n\n\
        👇 Bấm nút bên dưới để mở trang hướng dẫn chi tiết:";

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;

    Ok(())
}

async fn handle_referral_menu(
    bot: &Bot,
    chat_id: ChatId,
    telegram_id: i64,
    state: &SharedState,
) -> ResponseResult<()> {
    let Ok(Some(user)) = user_repo::find_by_telegram_id(&state.pool, telegram_id).await else {
        bot.send_message(chat_id, "Vui lòng gõ /start trước để khởi tạo tài khoản.")
            .await?;
        return Ok(());
    };

    let settings = settings_repo::get_settings(&state.pool).await.ok();
    let reward_vnd = settings
        .as_ref()
        .map(|s| s.referral_reward_vnd)
        .unwrap_or(state.config.referral_reward_vnd);
    let referred_bonus_vnd = settings
        .as_ref()
        .map(|s| s.referred_bonus_vnd)
        .unwrap_or(2_000);
    let min_first_deposit = settings
        .as_ref()
        .map(|s| s.referral_min_first_deposit_vnd)
        .unwrap_or(10_000);

    let summary = match referral_service::get_referral_summary(&state.pool, user.id).await {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(error = ?err, "Failed to load referral summary");
            bot.send_message(chat_id, "⚠️ Không thể tải thống kê mời bạn bè lúc này.")
                .await?;
            return Ok(());
        }
    };

    let bot_username = bot
        .get_me()
        .await
        .ok()
        .and_then(|me| me.username.clone())
        .unwrap_or_else(|| "your_bot".to_string());
    let ref_link = format!("https://t.me/{}?start=ref_{}", bot_username, user.ref_code);

    let text = messages::referral_dashboard_message(
        &ref_link,
        reward_vnd,
        referred_bonus_vnd,
        min_first_deposit,
        &summary,
    );

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboards::back_to_home_keyboard())
        .await?;

    Ok(())
}
