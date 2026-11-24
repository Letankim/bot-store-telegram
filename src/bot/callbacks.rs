use teloxide::{prelude::*, types::ParseMode};
use uuid::Uuid;

use crate::{
    bot::{
        commands::{
            handle_show_partner_shop, handle_show_plans, send_home_menu, send_vietqr_deposit_photo,
        },
        keyboards, messages,
    },
    domain::wallet::format_vnd,
    repo::{partner_repo, plan_repo, settings_repo, user_repo, wallet_repo},
    services::{partner_shop_service, payos_service::CheckPaymentStatus},
    state::SharedState,
};

pub async fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    state: SharedState,
) -> ResponseResult<()> {
    let _ = bot.answer_callback_query(&q.id).await;

    let Some(data) = q.data.as_deref() else {
        return Ok(());
    };
    let Some(msg) = q.message.as_ref() else {
        return Ok(());
    };
    let chat_id = msg.chat().id;
    let telegram_id = q.from.id.0 as i64;

    if data == "back:home" {
        state.awaiting_deposit_amount.write().await.remove(&telegram_id);
        send_home_menu(&bot, chat_id, telegram_id, &q.from.first_name, &state).await?;
        return Ok(());
    }

    if data == "back:plans" {
        state.awaiting_deposit_amount.write().await.remove(&telegram_id);
        handle_show_plans(&bot, chat_id, &state).await?;
        return Ok(());
    }

    if data == "pshop:list" {
        state.awaiting_deposit_amount.write().await.remove(&telegram_id);
        handle_show_partner_shop(&bot, chat_id, &state).await?;
        return Ok(());
    }

    if let Some(pid_str) = data.strip_prefix("pprod:") {
        if let Ok(product_id) = Uuid::parse_str(pid_str) {
            if let Ok(Some(product)) = partner_repo::find_product_by_id(&state.pool, product_id).await {
                let support_handle = settings_repo::get_settings(&state.pool)
                    .await
                    .map(|s| s.support_handle)
                    .unwrap_or_else(|_| state.config.support_handle.clone());
                let card = messages::partner_product_card(&product, &support_handle);
                bot.send_message(chat_id, card)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboards::partner_product_detail_keyboard(&product))
                    .await?;
            } else {
                bot.send_message(chat_id, "⚠️ Sản phẩm này không còn tồn tại.")
                    .reply_markup(keyboards::back_to_home_keyboard())
                    .await?;
            }
        }
        return Ok(());
    }

    if let Some(rest) = data.strip_prefix("pbuy:") {
        let mut parts = rest.split(':');
        let (Some(pid_str), Some(qty_str)) = (parts.next(), parts.next()) else {
            return Ok(());
        };
        let (Ok(product_id), Ok(qty)) = (Uuid::parse_str(pid_str), qty_str.parse::<i32>()) else {
            return Ok(());
        };
        let Ok(Some(product)) = partner_repo::find_product_by_id(&state.pool, product_id).await else {
            bot.send_message(chat_id, "⚠️ Sản phẩm không còn tồn tại.")
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
            return Ok(());
        };

        if product.is_maintenance {
            let text = messages::partner_product_maintenance_message(&product);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
            return Ok(());
        }

        if product.stock < qty {
            bot.send_message(
                chat_id,
                format!(
                    "❌ Sản phẩm này hiện chỉ còn <b>{}</b> trong kho, không đủ số lượng {} bạn chọn.",
                    product.stock, qty
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboards::back_to_home_keyboard())
            .await?;
            return Ok(());
        }

        let Ok(Some(user)) = user_repo::find_by_telegram_id(&state.pool, telegram_id).await else {
            bot.send_message(chat_id, "⚠️ Vui lòng bấm /start trước để khởi tạo ví.")
                .await?;
            return Ok(());
        };

        let total_price = product.sell_price_vnd.saturating_mul(i64::from(qty));
        let balance = wallet_repo::get_by_user_id(&state.pool, user.id)
            .await
            .map(|w| w.balance_vnd)
            .unwrap_or(0);

        if balance < total_price {
            let missing = total_price - balance;
            let settings = settings_repo::get_settings(&state.pool).await.ok();
            let min_deposit = settings.as_ref().map(|s| s.min_deposit_vnd).unwrap_or(10_000);
            let quick_amounts = settings
                .as_ref()
                .map(|s| s.parsed_quick_amounts())
                .unwrap_or_else(|| vec![40_000, 80_000, 150_000, 300_000]);
            let text = format!(
                "⚠️ <b>Số dư ví chưa đủ để mua hàng!</b>\n\n\
                 • Sản phẩm: <b>{}</b> x{qty}\n\
                 • Tổng tiền: <b>{} đ</b>\n\
                 • Số dư hiện có: <b>{} đ</b>\n\
                 • Còn thiếu: <b>{} đ</b>\n\n\
                 👇 Bấm nút bên dưới để tạo mã QR chuyển khoản (VietQR):",
                messages::escape_html(&product.name),
                format_vnd(total_price),
                format_vnd(balance),
                format_vnd(missing),
            );
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::payos_amount_keyboard(
                    Some(missing),
                    min_deposit,
                    &quick_amounts,
                ))
                .await?;
            return Ok(());
        }

        let _ = bot
            .send_message(chat_id, "⏳ Đang xuất kho giao hàng tự động, vui lòng đợi vài giây...")
            .await;

        match partner_shop_service::purchase_partner_product(&state, user, product_id, qty).await {
            Ok(outcome) => {
                let items_block = outcome
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| format!("{}. <code>{}</code>", i + 1, messages::escape_html(item)))
                    .collect::<Vec<_>>()
                    .join("\n");
                let text = format!(
                    "🎉 <b>MUA HÀNG THÀNH CÔNG!</b>\n\n\
                     • Mã đơn: <code>{}</code>\n\
                     • Sản phẩm: <b>{}</b> x{}\n\
                     • Đã thanh toán: <b>{} đ</b>\n\
                     • Số dư còn lại: <b>{} đ</b>\n\n\
                     🔑 <b>Thông tin sản phẩm (chạm để copy):</b>\n\
                     {}\n\n\
                     ✅ Cảm ơn bạn đã ủng hộ!",
                    outcome.order.order_code,
                    messages::escape_html(&outcome.order.product_name),
                    outcome.order.quantity,
                    format_vnd(outcome.order.total_sell_price_vnd),
                    format_vnd(outcome.remaining_balance_vnd),
                    items_block,
                );
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboards::back_to_home_keyboard())
                    .await?;
            }
            Err(err) => {
                bot.send_message(chat_id, format!("❌ {err}"))
                    .reply_markup(keyboards::back_to_home_keyboard())
                    .await?;
            }
        }
        return Ok(());
    }

    if data == "payos_custom" {
        let min_deposit = settings_repo::get_settings(&state.pool)
            .await
            .map(|s| s.min_deposit_vnd)
            .unwrap_or(10_000);
        state.awaiting_deposit_amount.write().await.insert(telegram_id);

        let text = format!(
            "✏️ <b>Nhập số tiền bạn muốn nạp (VND)</b>\n\n\
             • Số tiền nạp tối thiểu: <b>{} đ</b>\n\
             • Ví dụ: gõ <code>50000</code> hoặc <code>120000</code> rồi gửi ngay tại đây!\n\
             • Hệ thống sẽ lập tức tạo <b>mã QR VietQR</b> tương ứng cho bạn.",
            format_vnd(min_deposit)
        );
        bot.send_message(chat_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboards::back_to_home_keyboard())
            .await?;
        return Ok(());
    }

    if let Some(plan_id_str) = data.strip_prefix("plan:") {
        if let Ok(plan_id) = Uuid::parse_str(plan_id_str) {
            if let Ok(Some(plan)) = plan_repo::find_by_id(&state.pool, plan_id).await {
                let guide_url = state.config.guide_url(Some(plan_id_str));
                let support_handle = settings_repo::get_settings(&state.pool)
                    .await
                    .map(|s| s.support_handle)
                    .unwrap_or_else(|_| state.config.support_handle.clone());
                let card = messages::gemini_plan_card(&plan, &guide_url, &support_handle);
                bot.send_message(chat_id, card)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboards::plan_detail_keyboard(plan_id_str, &guide_url))
                    .await?;
            }
        }
        return Ok(());
    }

    if let Some(plan_id_str) = data.strip_prefix("chk_act:") {
        let Ok(plan_id) = Uuid::parse_str(plan_id_str) else {
            return Ok(());
        };
        let Ok(Some(plan)) = plan_repo::find_by_id(&state.pool, plan_id).await else {
            bot.send_message(chat_id, "⚠️ Gói dịch vụ không còn tồn tại.")
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
            return Ok(());
        };

        if plan.is_maintenance {
            let text = messages::plan_maintenance_message(&plan);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
            return Ok(());
        }

        if plan.available_stock <= 0 {
            bot.send_message(
                chat_id,
                "❌ Gói này hiện đang tạm hết tồn kho, vui lòng quay lại sau.",
            )
            .reply_markup(keyboards::back_to_home_keyboard())
            .await?;
            return Ok(());
        }

        let Ok(Some(user)) = user_repo::find_by_telegram_id(&state.pool, telegram_id).await else {
            bot.send_message(chat_id, "⚠️ Vui lòng bấm /start trước để khởi tạo ví.")
                .await?;
            return Ok(());
        };

        let balance = wallet_repo::get_by_user_id(&state.pool, user.id)
            .await
            .map(|w| w.balance_vnd)
            .unwrap_or(0);

        if balance < plan.price_vnd {
            let missing = plan.price_vnd - balance;
            let settings = settings_repo::get_settings(&state.pool).await.ok();
            let min_deposit = settings.as_ref().map(|s| s.min_deposit_vnd).unwrap_or(10_000);
            let quick_amounts = settings
                .as_ref()
                .map(|s| s.parsed_quick_amounts())
                .unwrap_or_else(|| vec![40_000, 80_000, 150_000, 300_000]);

            let text = format!(
                "⚠️ <b>Số dư ví chưa đủ để kích hoạt!</b>\n\n\
                 • Giá gói: <b>{} đ</b>\n\
                 • Số dư hiện có: <b>{} đ</b>\n\
                 • Còn thiếu: <b>{} đ</b>\n\
                 • Nạp tối thiểu mỗi lần: <b>{} đ</b>\n\n\
                 👇 Bấm nút bên dưới để tạo mã QR chuyển khoản (VietQR):",
                format_vnd(plan.price_vnd),
                format_vnd(balance),
                format_vnd(missing),
                format_vnd(min_deposit),
            );
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::payos_amount_keyboard(
                    Some(missing),
                    min_deposit,
                    &quick_amounts,
                ))
                .await?;
        } else {
            let activate_url = state.config.activate_url(plan_id_str);
            let guide_url = state.config.guide_url(Some(plan_id_str));
            let text = format!(
                "✅ <b>Số dư hợp lệ ({} đ)!</b>\n\n\
                 Gói đã chọn: <b>{}</b> (Giá: <b>{} đ</b>)\n\
                 👇 Bấm nút <b>Mở form điền Mail | Pass | 2FA</b> bên dưới để nhập thông tin kích hoạt:",
                format_vnd(balance),
                messages::escape_html(&plan.name),
                format_vnd(plan.price_vnd)
            );
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::open_activate_webapp_keyboard(
                    &activate_url,
                    &guide_url,
                ))
                .await?;
        }
        return Ok(());
    }

    if let Some(amt_str) = data.strip_prefix("payos_amt:") {
        if let Ok(amount_vnd) = amt_str.parse::<i64>() {
            send_vietqr_deposit_photo(&bot, chat_id, telegram_id, amount_vnd, &state).await?;
        }
        return Ok(());
    }

    if let Some(code_str) = data.strip_prefix("payos_chk:") {
        let Ok(order_code) = code_str.parse::<i64>() else {
            return Ok(());
        };
        match state
            .payos_service
            .check_and_confirm_payment(order_code)
            .await
        {
            Ok(CheckPaymentStatus::PaidNow(outcome)) => {
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

                let text = format!(
                    "✅ <b>Xác nhận thanh toán thành công!</b>\n\n\
                     • Mã giao dịch: <code>{}</code>\n\
                     • Đã cộng vào ví: <b>+{} đ</b>{}\n\
                     • Số dư hiện tại: <b>{} đ</b>\n\n\
                     🚀 Bấm /pixel để kích hoạt Gemini Pro ngay bây giờ!",
                    outcome.order_code,
                    format_vnd(outcome.amount_vnd),
                    bonus_note,
                    format_vnd(outcome.new_balance_vnd)
                );
                bot.send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboards::back_to_home_keyboard())
                    .await?;

                if let Some(ref_reward) = outcome.referral_reward {
                    let ref_msg = format!(
                        "🎉 <b>Bạn vừa nhận thưởng giới thiệu!</b>\n\n\
                         👤 <b>{}</b> vừa hoàn tất lần nạp tiền đầu tiên.\n\
                         💰 Đã cộng <b>+{} đ</b> vào ví của bạn!",
                        messages::escape_html(&ref_reward.referred_user_name),
                        format_vnd(ref_reward.reward_vnd),
                    );
                    let _ = bot
                        .send_message(ChatId(ref_reward.referrer_telegram_id), ref_msg)
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            }
            Ok(CheckPaymentStatus::AlreadyPaid {
                order_code,
                amount_vnd,
            }) => {
                bot.send_message(
                    chat_id,
                    format!(
                        "✅ Giao dịch <code>{}</code> (<b>{} đ</b>) đã được cộng tiền vào ví trước đó rồi!",
                        order_code,
                        format_vnd(amount_vnd)
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
            }
            Ok(CheckPaymentStatus::StillPending { order_code }) => {
                bot.send_message(
                    chat_id,
                    format!(
                        "⏳ Giao dịch <code>{}</code> hiện <b>chưa nhận được tiền</b>.\n\
                         Nếu bạn vừa chuyển khoản xong, vui lòng đợi 10–30 giây rồi bấm lại nút <b>✅ Tôi đã thanh toán</b> nhé!",
                        order_code
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
            Ok(CheckPaymentStatus::Cancelled { order_code }) => {
                bot.send_message(
                    chat_id,
                    format!("❌ Giao dịch <code>{}</code> đã bị hủy hoặc hết hạn.", order_code),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
            }
            Err(err) => {
                tracing::error!(error = ?err, "Error checking payment status");
                bot.send_message(chat_id, format!("⚠️ Chưa thể kiểm tra lúc này: {err}"))
                    .await?;
            }
        }
        return Ok(());
    }

    if let Some(code_str) = data.strip_prefix("payos_cancel:") {
        let Ok(order_code) = code_str.parse::<i64>() else {
            return Ok(());
        };
        let Ok(Some(user)) = user_repo::find_by_telegram_id(&state.pool, telegram_id).await else {
            return Ok(());
        };
        match state
            .payos_service
            .cancel_payment(order_code, user.id)
            .await
        {
            Ok(true) => {
                bot.send_message(
                    chat_id,
                    format!(
                        "❌ Đã hủy yêu cầu nạp tiền <code>{}</code> thành công.\n\
                         Bạn có thể chọn lại mức nạp khác bất cứ lúc nào!",
                        order_code
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
            }
            Ok(false) => {
                bot.send_message(
                    chat_id,
                    format!(
                        "ℹ️ Giao dịch <code>{}</code> đã hoàn tất hoặc đã được hủy trước đó.",
                        order_code
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboards::back_to_home_keyboard())
                .await?;
            }
            Err(err) => {
                bot.send_message(chat_id, format!("⚠️ Không thể hủy giao dịch: {err}"))
                    .await?;
            }
        }
    }

    Ok(())
}
