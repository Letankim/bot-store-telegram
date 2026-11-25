use teloxide::{
    prelude::*,
    types::{
        BotCommand, InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup,
        ParseMode, ReplyMarkup, Update,
    },
};
use url::Url;
use uuid::Uuid;

use crate::{
    bot::{keyboards, messages, messages::escape_html},
    crypto::constant_time_eq_str,
    domain::wallet::format_vnd,
    repo::{activation_repo, plan_repo, settings_repo, user_repo, wallet_repo},
    services::{partner_shop_service, smm_service},
    state::{AdminBotPendingAction, SharedState},
};

const BTN_DASHBOARD: &str = "📊 Tổng quan";
const BTN_PENDING_ORDERS: &str = "📥 Đơn chờ xử lý";
const BTN_PLANS: &str = "📦 Quản lý gói & bảo trì";
const BTN_BROADCAST: &str = "📢 Gửi thông báo khách";
const BTN_USERS: &str = "👥 Tìm user & ví";
const BTN_SETTINGS: &str = "⚙️ Cấu hình & API";
const BTN_LOGOUT: &str = "🔒 Đăng xuất";

/// Register slash commands on the dedicated Admin Bot.
pub async fn register_commands(admin_bot: &Bot) {
    let cmds = vec![
        BotCommand::new("start", "🔐 Mở bảng điều khiển quản trị"),
        BotCommand::new("cancel", "❌ Hủy thao tác đang nhập"),
        BotCommand::new("logout", "🔒 Đăng xuất & khóa bot quản trị"),
    ];
    if let Err(err) = admin_bot.set_my_commands(cmds).await {
        tracing::warn!(error = ?err, "Failed to register Admin Bot slash commands");
    }
}

/// Dispatch a single Telegram Update for the dedicated Admin Bot (Webhook & Polling).
pub async fn dispatch_update(admin_bot: Bot, update: Update, state: SharedState) {
    match update.kind {
        teloxide::types::UpdateKind::Message(msg) => {
            if let Err(err) = handle_message(admin_bot, msg, state).await {
                tracing::error!(error = ?err, "Error handling Admin Bot message");
            }
        }
        teloxide::types::UpdateKind::CallbackQuery(q) => {
            if let Err(err) = handle_callback(admin_bot, q, state).await {
                tracing::error!(error = ?err, "Error handling Admin Bot callback");
            }
        }
        _ => {}
    }
}

/// Run long-polling loop for the dedicated Admin Bot (`BOT_MODE=polling`).
pub async fn run_polling(admin_bot: Bot, state: SharedState) {
    register_commands(&admin_bot).await;

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    Dispatcher::builder(admin_bot, handler)
        .dependencies(dptree::deps![state])
        .build()
        .dispatch()
        .await;
}

fn admin_reply_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![
        vec![
            KeyboardButton::new(BTN_DASHBOARD),
            KeyboardButton::new(BTN_PENDING_ORDERS),
        ],
        vec![
            KeyboardButton::new(BTN_PLANS),
            KeyboardButton::new(BTN_BROADCAST),
        ],
        vec![
            KeyboardButton::new(BTN_USERS),
            KeyboardButton::new(BTN_SETTINGS),
        ],
        vec![KeyboardButton::new(BTN_LOGOUT)],
    ])
    .resize_keyboard()
    .persistent()
}

async fn is_authenticated(state: &SharedState, telegram_id: i64) -> bool {
    state.admin_bot_sessions.read().await.contains(&telegram_id)
}

async fn send_login_prompt(admin_bot: &Bot, chat_id: ChatId, wrong_key: bool) -> ResponseResult<()> {
    let text = if wrong_key {
        "⛔ <b>Khóa quản trị không chính xác!</b>\n\n\
         Vui lòng nhập lại đúng <code>ADMIN_API_KEY</code> để đăng nhập:"
    } else {
        "🔐 <b>ĐĂNG NHẬP BOT QUẢN TRỊ GEMINI PRO</b>\n\
         ══════════════════════════\n\n\
         Vui lòng nhập khóa <code>ADMIN_API_KEY</code> ngay tại đây để mở khóa toàn bộ bảng điều khiển quản trị.\n\
         <i>(Tin nhắn chứa khóa sẽ được tự động xóa ngay sau khi xác thực để bảo mật.)</i>"
    };

    admin_bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(ReplyMarkup::kb_remove())
        .await?;
    Ok(())
}

async fn handle_message(admin_bot: Bot, msg: Message, state: SharedState) -> ResponseResult<()> {
    let Some(from) = msg.from.as_ref() else {
        return Ok(());
    };
    let telegram_id = from.id.0 as i64;
    let chat_id = msg.chat.id;
    let text = msg.text().or_else(|| msg.caption()).unwrap_or("").trim();

    // 1. Auth gate — require ADMIN_API_KEY before any admin action
    if !is_authenticated(&state, telegram_id).await {
        if text.is_empty() || text.starts_with("/start") {
            send_login_prompt(&admin_bot, chat_id, false).await?;
            return Ok(());
        }

        // Delete the user's message containing the key so it doesn't stay in chat history
        let _ = admin_bot.delete_message(chat_id, msg.id).await;

        if constant_time_eq_str(text, &state.config.admin_api_key) {
            state.admin_bot_sessions.write().await.insert(telegram_id);
            admin_bot
                .send_message(
                    chat_id,
                    "✅ <b>Đăng nhập quản trị thành công!</b>\n\n\
                     Chào mừng bạn đến với <b>Bot Quản Trị Gemini Pro</b>.\n\
                     👇 Sử dụng thanh menu bên dưới để điều hành toàn bộ hệ thống trực tiếp trên Telegram:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(admin_reply_keyboard())
                .await?;
            send_dashboard(&admin_bot, chat_id, &state).await?;
        } else {
            send_login_prompt(&admin_bot, chat_id, true).await?;
        }
        return Ok(());
    }

    // 2. Global commands for authenticated admin
    if text.starts_with("/logout") || text == BTN_LOGOUT {
        state.admin_bot_sessions.write().await.remove(&telegram_id);
        state.admin_bot_pending.write().await.remove(&telegram_id);
        admin_bot
            .send_message(
                chat_id,
                "🔒 <b>Đã đăng xuất &amp; khóa Bot quản trị!</b>\n\
                 Gõ /start hoặc nhập <code>ADMIN_API_KEY</code> khi cần đăng nhập lại.",
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(ReplyMarkup::kb_remove())
            .await?;
        return Ok(());
    }

    if text.starts_with("/cancel") {
        state.admin_bot_pending.write().await.remove(&telegram_id);
        admin_bot
            .send_message(chat_id, "❌ Đã hủy thao tác đang nhập.")
            .reply_markup(admin_reply_keyboard())
            .await?;
        return Ok(());
    }

    if text.starts_with("/start") || text == BTN_DASHBOARD {
        state.admin_bot_pending.write().await.remove(&telegram_id);
        send_dashboard(&admin_bot, chat_id, &state).await?;
        return Ok(());
    }

    // 3. Reply keyboard main menu buttons (always cancel any pending prompt)
    match text {
        BTN_PENDING_ORDERS => {
            state.admin_bot_pending.write().await.remove(&telegram_id);
            send_pending_orders(&admin_bot, chat_id, &state).await?;
            return Ok(());
        }
        BTN_PLANS => {
            state.admin_bot_pending.write().await.remove(&telegram_id);
            send_plans_manager(&admin_bot, chat_id, &state).await?;
            return Ok(());
        }
        BTN_BROADCAST => {
            state.admin_bot_pending.write().await.remove(&telegram_id);
            send_broadcast_menu(&admin_bot, chat_id).await?;
            return Ok(());
        }
        BTN_USERS => {
            state
                .admin_bot_pending
                .write()
                .await
                .insert(telegram_id, AdminBotPendingAction::SearchUser);
            admin_bot
                .send_message(
                    chat_id,
                    "👥 <b>TÌM NGƯỜI DÙNG &amp; QUẢN LÝ VÍ</b>\n\n\
                     Nhập <b>Telegram ID</b>, <b>@username</b>, hoặc <b>tên khách hàng</b> để tìm kiếm.\n\
                     <i>(Hoặc gõ <code>all</code> để xem 5 khách hàng mới nhất, /cancel để hủy)</i>",
                )
                .parse_mode(ParseMode::Html)
                .await?;
            return Ok(());
        }
        BTN_SETTINGS => {
            state.admin_bot_pending.write().await.remove(&telegram_id);
            send_settings_panel(&admin_bot, chat_id, &state).await?;
            return Ok(());
        }
        _ => {}
    }

    // 4. Handle pending multi-step inputs
    let pending = state.admin_bot_pending.write().await.remove(&telegram_id);
    if let Some(action) = pending {
        handle_pending_input(&admin_bot, chat_id, telegram_id, text, action, &state).await?;
        return Ok(());
    }

    // 5. Auto-detect forwarded/pasted partner stock or price alert messages!
    if partner_shop_service::parse_partner_bot_alert(text).is_some() {
        match partner_shop_service::ingest_partner_alert_message(&state, text, true).await {
            Ok(out) => {
                let tag = if out.created { "🆕 Đã tạo mới" } else { "🔄 Đã cập nhật" };
                admin_bot
                    .send_message(
                        chat_id,
                        format!(
                            "✅ <b>{tag} sản phẩm tự động!</b>\n\n\
                             • Tên: <b>{}</b> (<code>{}</code>)\n\
                             • Giá gốc: <b>{}đ</b> + lời <b>{}đ</b> = <b>{}đ</b>\n\
                             • Tồn kho: <b>{}</b>\n\
                             • Gửi thông báo khách: <b>{}</b>",
                            escape_html(&out.plan.name),
                            out.plan.code,
                            format_vnd(out.partner_price_vnd),
                            format_vnd(out.markup_vnd),
                            format_vnd(out.sell_price_vnd),
                            out.plan.available_stock,
                            if out.broadcast_triggered { "Có" } else { "Không" },
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            Err(err) => {
                admin_bot
                    .send_message(chat_id, format!("❌ Không thể nhập sản phẩm tự động: {err}"))
                    .await?;
            }
        }
        return Ok(());
    }

    // Default fallback
    admin_bot
        .send_message(
            chat_id,
            "👇 Vui lòng chọn chức năng trên thanh menu quản trị bên dưới:",
        )
        .reply_markup(admin_reply_keyboard())
        .await?;
    Ok(())
}

async fn handle_pending_input(
    admin_bot: &Bot,
    chat_id: ChatId,
    telegram_id: i64,
    text: &str,
    action: AdminBotPendingAction,
    state: &SharedState,
) -> ResponseResult<()> {
    match action {
        AdminBotPendingAction::RefundOrder {
            order_id,
            order_code,
        } => {
            if text.is_empty() {
                admin_bot
                    .send_message(chat_id, "⚠️ Lý do hoàn tiền không được để trống.")
                    .await?;
                return Ok(());
            }
            match state
                .activation_service
                .refund_failed_order(order_id, text)
                .await
            {
                Ok(outcome) => {
                    let new_bal = outcome.refunded_balance_vnd.unwrap_or(0);
                    let plan_name = outcome.plan_name.as_deref().unwrap_or("Gemini Pro");
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
                        escape_html(text),
                    );

                    let mut sent_ok = false;
                    if let Some(cust_tg) = outcome.user_telegram_id {
                        if state
                            .bot
                            .send_message(ChatId(cust_tg), &customer_msg)
                            .parse_mode(ParseMode::Html)
                            .await
                            .is_ok()
                        {
                            sent_ok = true;
                        }
                    }

                    let delivery = if sent_ok {
                        "✅ Đã gửi tới khách thành công"
                    } else {
                        "⚠️ Chưa gửi được tới khách"
                    };
                    let report = format!(
                        "🛡️ <b>ĐÃ HOÀN TIỀN ĐƠN <code>{}</code></b>\n\n\
                         • Khách hàng: <b>{}</b>\n\
                         • Hoàn tiền: <b>+{} đ</b> (Số dư mới: <b>{} đ</b>)\n\
                         • Trạng thái gửi khách: {}\n\n\
                         📩 <b>Nội dung khách đã nhận:</b>\n\
                         <blockquote>{}</blockquote>",
                        order_code,
                        escape_html(outcome.user_full_name.as_deref().unwrap_or("Khách hàng")),
                        format_vnd(outcome.order.price_paid_vnd),
                        format_vnd(new_bal),
                        delivery,
                        customer_msg,
                    );
                    admin_bot
                        .send_message(chat_id, report)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(err) => {
                    admin_bot
                        .send_message(chat_id, format!("❌ Không thể hoàn tiền đơn {order_code}: {err}"))
                        .await?;
                }
            }
        }

        AdminBotPendingAction::AddPlanStock { plan_id, plan_name } => {
            let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            let Ok(add_qty) = digits.parse::<i32>() else {
                admin_bot
                    .send_message(
                        chat_id,
                        "⚠️ Vui lòng chỉ nhập số nguyên dương (ví dụ: <code>20</code>).",
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                state
                    .admin_bot_pending
                    .write()
                    .await
                    .insert(telegram_id, AdminBotPendingAction::AddPlanStock { plan_id, plan_name });
                return Ok(());
            };
            if add_qty <= 0 {
                admin_bot
                    .send_message(chat_id, "⚠️ Số lượng cộng thêm phải lớn hơn 0.")
                    .await?;
                return Ok(());
            }

            let Ok(Some(plan)) = plan_repo::find_by_id(&state.pool, plan_id).await else {
                admin_bot
                    .send_message(chat_id, "⚠️ Không tìm thấy gói dịch vụ.")
                    .await?;
                return Ok(());
            };
            let new_stock = plan.available_stock + add_qty;
            match plan_repo::update_plan_full(
                &state.pool,
                plan_id,
                None,
                None,
                None,
                Some(new_stock),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            {
                Ok(updated) => {
                    admin_bot
                        .send_message(
                            chat_id,
                            format!(
                                "✅ Đã cộng <b>+{add_qty}</b> tồn kho cho <b>{}</b> (Kho hiện tại: <b>{}</b>).\n\
                                 📢 Đang gửi thông báo \"HÀNG MỚI VỪA LÊN KHO\" tới tất cả khách hàng...",
                                escape_html(&updated.name),
                                updated.available_stock
                            ),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;

                    if updated.is_active && !updated.is_maintenance {
                        let text_bc = messages::restock_broadcast_message(&updated, add_qty);
                        let kb = keyboards::restock_broadcast_keyboard(&updated.id.to_string());
                        spawn_broadcast_with_report(
                            state.clone(),
                            admin_bot.clone(),
                            chat_id,
                            text_bc,
                            kb,
                        );
                    }
                }
                Err(err) => {
                    admin_bot
                        .send_message(chat_id, format!("❌ Lỗi cập nhật tồn kho: {err}"))
                        .await?;
                }
            }
        }

        AdminBotPendingAction::BroadcastCustom => {
            if text.is_empty() {
                admin_bot
                    .send_message(chat_id, "⚠️ Nội dung thông báo không được để trống.")
                    .await?;
                return Ok(());
            }
            let formatted = format!(
                "📢 <b>THÔNG BÁO TỪ HỆ THỐNG</b>\n\
                 ══════════════════════════\n\n\
                 {}",
                escape_html(text)
            );
            admin_bot
                .send_message(chat_id, "🚀 Đang gửi thông báo tới tất cả khách hàng...")
                .await?;
            spawn_broadcast_with_report(
                state.clone(),
                admin_bot.clone(),
                chat_id,
                formatted,
                keyboards::maintenance_broadcast_keyboard(),
            );
        }

        AdminBotPendingAction::SearchUser => {
            let search = if text.eq_ignore_ascii_case("all") {
                None
            } else {
                Some(text.trim_start_matches('@'))
            };
            match user_repo::list_users_with_wallets(&state.pool, search, 5, 0).await {
                Ok((users, total)) => {
                    if users.is_empty() {
                        admin_bot
                            .send_message(chat_id, "🔍 Không tìm thấy người dùng nào khớp với từ khóa.")
                            .await?;
                        return Ok(());
                    }
                    admin_bot
                        .send_message(
                            chat_id,
                            format!("👥 Tìm thấy <b>{total}</b> người dùng (hiển thị tối đa 5):"),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;

                    for u in users {
                        let status_txt = if u.is_banned { "🚫 Đã khóa" } else { "🟢 Hoạt động" };
                        let uname = u
                            .username
                            .as_deref()
                            .map(|s| format!(" (@{})", escape_html(s)))
                            .unwrap_or_default();
                        let card = format!(
                            "👤 <b>{}</b>{}\n\
                             • Telegram ID: <code>{}</code>\n\
                             • Trạng thái: <b>{}</b>\n\
                             • Số dư ví: <b>{} đ</b>\n\
                             • Tổng nạp: <b>{} đ</b>\n\
                             • Đơn hàng: <b>{}</b> • Đã mời: <b>{}</b>",
                            escape_html(&u.full_name),
                            uname,
                            u.telegram_id,
                            status_txt,
                            format_vnd(u.balance_vnd),
                            format_vnd(u.total_deposited_vnd),
                            u.orders_count,
                            u.referrals_count,
                        );
                        let ban_btn = if u.is_banned {
                            InlineKeyboardButton::callback("🔓 Mở khóa", format!("adm:ban:{}", u.id))
                        } else {
                            InlineKeyboardButton::callback("🚫 Khóa user", format!("adm:ban:{}", u.id))
                        };
                        let kb = InlineKeyboardMarkup::new(vec![
                            vec![
                                InlineKeyboardButton::callback("➕ Cộng tiền", format!("adm:wal:+:{}", u.id)),
                                InlineKeyboardButton::callback("➖ Trừ tiền", format!("adm:wal:-:{}", u.id)),
                            ],
                            vec![ban_btn],
                        ]);
                        admin_bot
                            .send_message(chat_id, card)
                            .parse_mode(ParseMode::Html)
                            .reply_markup(kb)
                            .await?;
                    }
                }
                Err(err) => {
                    admin_bot
                        .send_message(chat_id, format!("❌ Lỗi tìm kiếm user: {err}"))
                        .await?;
                }
            }
        }

        AdminBotPendingAction::AdjustUserWallet {
            user_id,
            user_name,
            sign,
        } => {
            let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            let Ok(amt) = digits.parse::<i64>() else {
                admin_bot
                    .send_message(
                        chat_id,
                        "⚠️ Số tiền không hợp lệ. Vui lòng chỉ nhập số (ví dụ: <code>50000</code>).",
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                state.admin_bot_pending.write().await.insert(
                    telegram_id,
                    AdminBotPendingAction::AdjustUserWallet {
                        user_id,
                        user_name,
                        sign,
                    },
                );
                return Ok(());
            };
            if amt <= 0 {
                admin_bot
                    .send_message(chat_id, "⚠️ Số tiền phải lớn hơn 0.")
                    .await?;
                return Ok(());
            }

            let Ok(Some(user)) = user_repo::find_by_id(&state.pool, user_id).await else {
                admin_bot
                    .send_message(chat_id, "⚠️ Không tìm thấy người dùng.")
                    .await?;
                return Ok(());
            };
            let Ok(mut tx) = state.pool.begin().await else {
                admin_bot
                    .send_message(chat_id, "⚠️ Lỗi mở giao dịch DB.")
                    .await?;
                return Ok(());
            };
            let Ok(wallet) = wallet_repo::lock_by_user_id(&mut tx, user.id).await else {
                admin_bot
                    .send_message(chat_id, "⚠️ Không tìm thấy ví người dùng.")
                    .await?;
                return Ok(());
            };

            let idem_key = format!("admin_bot_adj_{}", Uuid::new_v4().simple());
            let desc = if sign > 0 {
                "Admin cộng tiền qua Bot quản trị"
            } else {
                "Admin trừ tiền qua Bot quản trị"
            };

            let updated_res = if sign > 0 {
                wallet_repo::credit_wallet(
                    &mut tx,
                    &wallet,
                    amt,
                    false,
                    "ADMIN_ADJUST",
                    &idem_key,
                    desc,
                )
                .await
            } else {
                wallet_repo::debit_wallet(&mut tx, &wallet, amt, "ADMIN_ADJUST", &idem_key, desc)
                    .await
            };

            match updated_res {
                Ok(updated_wallet) => {
                    if tx.commit().await.is_err() {
                        admin_bot
                            .send_message(chat_id, "❌ Lỗi lưu giao dịch ví.")
                            .await?;
                        return Ok(());
                    }

                    let sign_str = if sign > 0 { "+" } else { "-" };
                    let cust_notice = format!(
                        "{} <b>Thông báo biến động số dư ví</b>\n\n\
                         • Số tiền: <b>{}{} đ</b>\n\
                         • Nội dung: <i>{}</i>\n\
                         • Số dư hiện tại: <b>{} đ</b>",
                        if sign > 0 { "💰" } else { "💸" },
                        sign_str,
                        format_vnd(amt),
                        desc,
                        format_vnd(updated_wallet.balance_vnd),
                    );
                    let _ = state
                        .bot
                        .send_message(ChatId(user.telegram_id), cust_notice)
                        .parse_mode(ParseMode::Html)
                        .await;

                    admin_bot
                        .send_message(
                            chat_id,
                            format!(
                                "✅ Đã {} <b>{} đ</b> cho <b>{}</b>.\n\
                                 • Số dư mới: <b>{} đ</b>\n\
                                 • Đã gửi thông báo biến động số dư cho khách!",
                                if sign > 0 { "cộng" } else { "trừ" },
                                format_vnd(amt),
                                escape_html(&user_name),
                                format_vnd(updated_wallet.balance_vnd)
                            ),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(err) => {
                    admin_bot
                        .send_message(chat_id, format!("❌ Không thể điều chỉnh ví: {err}"))
                        .await?;
                }
            }
        }
    }
    Ok(())
}

async fn handle_callback(
    admin_bot: Bot,
    q: CallbackQuery,
    state: SharedState,
) -> ResponseResult<()> {
    let _ = admin_bot.answer_callback_query(&q.id).await;
    let Some(data) = q.data.as_deref() else {
        return Ok(());
    };
    let Some(msg) = q.message.as_ref() else {
        return Ok(());
    };
    let chat_id = msg.chat().id;
    let telegram_id = q.from.id.0 as i64;

    if !is_authenticated(&state, telegram_id).await {
        send_login_prompt(&admin_bot, chat_id, false).await?;
        return Ok(());
    }

    if data == "adm:dash" {
        send_dashboard(&admin_bot, chat_id, &state).await?;
        return Ok(());
    }

    if data == "adm:orders" {
        send_pending_orders(&admin_bot, chat_id, &state).await?;
        return Ok(());
    }

    if data == "adm:plans" {
        send_plans_manager(&admin_bot, chat_id, &state).await?;
        return Ok(());
    }

    // Push order to upstream API
    if let Some(id_str) = data.strip_prefix("adm:push:") {
        let Ok(order_id) = Uuid::parse_str(id_str) else {
            return Ok(());
        };
        let Ok(settings) = settings_repo::get_settings(&state.pool).await else {
            return Ok(());
        };
        match state.activation_service.load_decrypted_order(order_id).await {
            Ok((order, creds, user_tg_id, plan_api_mode)) => {
                if order.status != "PENDING" {
                    admin_bot
                        .send_message(
                            chat_id,
                            format!("⚠️ Đơn <code>{}</code> đang ở trạng thái <b>{}</b>.", order.order_code, order.status),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                    return Ok(());
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
                                settings.smm_api_base_url,
                                settings.smm_api_key,
                            );
                        }
                        admin_bot
                            .send_message(
                                chat_id,
                                format!(
                                    "🚀 Đã đẩy đơn <code>{}</code> sang API thành công!\n• Job ID: <code>#{}</code> • Mode: <b>{}</b>",
                                    order.order_code, job.job_id, job.mode
                                ),
                            )
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                    Err(err) => {
                        admin_bot
                            .send_message(
                                chat_id,
                                format!("❌ Lỗi đẩy đơn <code>{}</code> sang API: {}", order.order_code, escape_html(&err.to_string())),
                            )
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                }
            }
            Err(err) => {
                admin_bot
                    .send_message(chat_id, format!("❌ Không tải được đơn: {err}"))
                    .await?;
            }
        }
        return Ok(());
    }

    // Complete order manually
    if let Some(id_str) = data.strip_prefix("adm:done:") {
        let Ok(order_id) = Uuid::parse_str(id_str) else {
            return Ok(());
        };
        match state
            .activation_service
            .complete_order(order_id, Some("Đã kích hoạt thành công!"))
            .await
        {
            Ok(outcome) => {
                if let Some(tg_id) = outcome.user_telegram_id {
                    let msg_cust = format!(
                        "🎉 <b>Đơn kích hoạt Gemini Pro đã hoàn tất!</b>\n\n\
                         • Mã đơn: <code>{}</code>\n\
                         • Gmail: <code>{}</code>\n\n\
                         ✅ Bạn đã có thể đăng nhập Gemini Pro và sử dụng ngay. Cảm ơn bạn đã ủng hộ!",
                        outcome.order.order_code,
                        escape_html(&outcome.order.gmail_masked),
                    );
                    let _ = state
                        .bot
                        .send_message(ChatId(tg_id), msg_cust)
                        .parse_mode(ParseMode::Html)
                        .await;
                }
                admin_bot
                    .send_message(
                        chat_id,
                        format!("✅ Đã đánh dấu hoàn tất đơn <code>{}</code> và báo khách!", outcome.order.order_code),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            Err(err) => {
                admin_bot
                    .send_message(chat_id, format!("❌ Không thể hoàn tất đơn: {err}"))
                    .await?;
            }
        }
        return Ok(());
    }

    // Prompt refund reason
    if let Some(id_str) = data.strip_prefix("adm:ref:") {
        let Ok(order_id) = Uuid::parse_str(id_str) else {
            return Ok(());
        };
        if let Ok(Some(order)) = activation_repo::find_by_id(&state.pool, order_id).await {
            state.admin_bot_pending.write().await.insert(
                telegram_id,
                AdminBotPendingAction::RefundOrder {
                    order_id,
                    order_code: order.order_code.clone(),
                },
            );
            admin_bot
                .send_message(
                    chat_id,
                    format!(
                        "↩️ <b>Hoàn tiền 100% cho đơn <code>{}</code></b>\n\n\
                         Nhập <b>lý do hoàn tiền</b> để gửi cho khách hàng:\n\
                         <i>(VD: Gmail chưa đóng Hồ sơ thanh toán hoặc sai mật khẩu — gõ /cancel để hủy)</i>",
                        order.order_code
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
        }
        return Ok(());
    }

    // Toggle plan maintenance + broadcast
    if let Some(id_str) = data.strip_prefix("adm:maint:") {
        let Ok(plan_id) = Uuid::parse_str(id_str) else {
            return Ok(());
        };
        if let Ok(Some(plan)) = plan_repo::find_by_id(&state.pool, plan_id).await {
            let new_maint = !plan.is_maintenance;
            if let Ok(updated) = plan_repo::update_plan_full(
                &state.pool,
                plan_id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(new_maint),
                None,
            )
            .await
            {
                let status_str = if new_maint {
                    "🔧 ĐÃ BẬT BẢO TRÌ"
                } else {
                    "✅ ĐÃ TẮT BẢO TRÌ (MỞ BÁN LẠI)"
                };
                admin_bot
                    .send_message(
                        chat_id,
                        format!(
                            "{status_str} cho gói <b>{}</b>!\n📢 Đang gửi thông báo tới tất cả khách hàng...",
                            escape_html(&updated.name)
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;

                if updated.is_active {
                    let (bc_text, kb) = if new_maint {
                        (
                            messages::maintenance_started_broadcast_message(&updated),
                            keyboards::maintenance_broadcast_keyboard(),
                        )
                    } else {
                        (
                            messages::maintenance_ended_broadcast_message(&updated),
                            keyboards::restock_broadcast_keyboard(&updated.id.to_string()),
                        )
                    };
                    spawn_broadcast_with_report(
                        state.clone(),
                        admin_bot.clone(),
                        chat_id,
                        bc_text,
                        kb,
                    );
                }
            }
        }
        return Ok(());
    }

    // Toggle plan active visibility
    if let Some(id_str) = data.strip_prefix("adm:active:") {
        let Ok(plan_id) = Uuid::parse_str(id_str) else {
            return Ok(());
        };
        if let Ok(Some(plan)) = plan_repo::find_by_id(&state.pool, plan_id).await {
            let new_active = !plan.is_active;
            if let Ok(updated) = plan_repo::update_plan_full(
                &state.pool,
                plan_id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(new_active),
                None,
                None,
            )
            .await
            {
                admin_bot
                    .send_message(
                        chat_id,
                        format!(
                            "{} gói <b>{}</b>.",
                            if new_active { "🟢 Đã hiện lại" } else { "⛔ Đã tạm ẩn" },
                            escape_html(&updated.name)
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        }
        return Ok(());
    }

    // Prompt add plan stock
    if let Some(id_str) = data.strip_prefix("adm:stock:") {
        let Ok(plan_id) = Uuid::parse_str(id_str) else {
            return Ok(());
        };
        if let Ok(Some(plan)) = plan_repo::find_by_id(&state.pool, plan_id).await {
            state.admin_bot_pending.write().await.insert(
                telegram_id,
                AdminBotPendingAction::AddPlanStock {
                    plan_id,
                    plan_name: plan.name.clone(),
                },
            );
            admin_bot
                .send_message(
                    chat_id,
                    format!(
                        "➕ <b>Cộng tồn kho cho {}</b> (hiện có: <b>{}</b>)\n\n\
                         Nhập số lượng muốn cộng thêm (VD: <code>20</code>) — hệ thống sẽ tự gửi thông báo lên kho tới khách hàng!\n\
                         <i>(Gõ /cancel để hủy)</i>",
                        escape_html(&plan.name),
                        plan.available_stock
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
        }
        return Ok(());
    }

    // Broadcast presets & custom
    if data == "adm:bc:maint" {
        let text = "🔧 <b>THÔNG BÁO BẢO TRÌ HỆ THỐNG</b>\n\
                    ══════════════════════════\n\n\
                    Hệ thống hiện đang bảo trì nâng cấp để hoạt động ổn định hơn.\n\
                    Vui lòng thử lại sau — hệ thống sẽ thông báo ngay khi hoàn tất!";
        admin_bot
            .send_message(chat_id, "🚀 Đang gửi thông báo bảo trì tới tất cả khách hàng...")
            .await?;
        spawn_broadcast_with_report(
            state.clone(),
            admin_bot.clone(),
            chat_id,
            text.to_string(),
            keyboards::maintenance_broadcast_keyboard(),
        );
        return Ok(());
    }

    if data == "adm:bc:done" {
        let text = "✅ <b>HỆ THỐNG ĐÃ BẢO TRÌ XONG</b>\n\
                    ══════════════════════════\n\n\
                    Quá trình bảo trì đã hoàn tất! Tất cả dịch vụ đã hoạt động bình thường trở lại.\n\
                    Bạn có thể đặt đơn kích hoạt ngay bây giờ bằng lệnh /pixel nhé!";
        admin_bot
            .send_message(chat_id, "🚀 Đang gửi thông báo mở bán lại tới tất cả khách hàng...")
            .await?;
        spawn_broadcast_with_report(
            state.clone(),
            admin_bot.clone(),
            chat_id,
            text.to_string(),
            keyboards::maintenance_broadcast_keyboard(),
        );
        return Ok(());
    }

    if data == "adm:bc:custom" {
        state
            .admin_bot_pending
            .write()
            .await
            .insert(telegram_id, AdminBotPendingAction::BroadcastCustom);
        admin_bot
            .send_message(
                chat_id,
                "✏️ <b>SOẠN THÔNG BÁO GỬI TẤT CẢ KHÁCH HÀNG</b>\n\n\
                 Nhập nội dung bạn muốn gửi ngay tại đây (hệ thống sẽ gửi tới toàn bộ user đang hoạt động).\n\
                 <i>(Gõ /cancel để hủy)</i>",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    // Wallet adjust prompt
    if let Some(rest) = data.strip_prefix("adm:wal:") {
        let (sign, id_str) = if let Some(id) = rest.strip_prefix("+:") {
            (1_i64, id)
        } else if let Some(id) = rest.strip_prefix("-:") {
            (-1_i64, id)
        } else {
            return Ok(());
        };
        let Ok(user_id) = Uuid::parse_str(id_str) else {
            return Ok(());
        };
        if let Ok(Some(user)) = user_repo::find_by_id(&state.pool, user_id).await {
            state.admin_bot_pending.write().await.insert(
                telegram_id,
                AdminBotPendingAction::AdjustUserWallet {
                    user_id,
                    user_name: user.full_name.clone(),
                    sign,
                },
            );
            admin_bot
                .send_message(
                    chat_id,
                    format!(
                        "{} <b>{} cho {}</b>\n\n\
                         Nhập số tiền VND (VD: <code>50000</code>):\n\
                         <i>(Gõ /cancel để hủy)</i>",
                        if sign > 0 { "➕" } else { "➖" },
                        if sign > 0 { "Cộng tiền ví" } else { "Trừ tiền ví" },
                        escape_html(&user.full_name),
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
        }
        return Ok(());
    }

    // Ban / unban user
    if let Some(id_str) = data.strip_prefix("adm:ban:") {
        let Ok(user_id) = Uuid::parse_str(id_str) else {
            return Ok(());
        };
        if let Ok(Some(user)) = user_repo::find_by_id(&state.pool, user_id).await {
            let next_ban = !user.is_banned;
            if user_repo::set_user_banned(&state.pool, user_id, next_ban)
                .await
                .is_ok()
            {
                admin_bot
                    .send_message(
                        chat_id,
                        format!(
                            "{} tài khoản <b>{}</b> (ID: <code>{}</code>).",
                            if next_ban { "🚫 Đã khóa" } else { "🔓 Đã mở khóa" },
                            escape_html(&user.full_name),
                            user.telegram_id
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        }
        return Ok(());
    }

    // Toggle settings
    if data == "adm:cfg:autopush" || data == "adm:cfg:autorefund" {
        if let Ok(s) = settings_repo::get_settings(&state.pool).await {
            let new_auto_push = if data == "adm:cfg:autopush" {
                !s.smm_auto_push
            } else {
                s.smm_auto_push
            };
            let new_auto_refund = if data == "adm:cfg:autorefund" {
                !s.smm_auto_refund_on_fail
            } else {
                s.smm_auto_refund_on_fail
            };
            let _ = settings_repo::update_settings(
                &state.pool,
                &s.support_handle,
                s.min_deposit_vnd,
                s.referral_reward_vnd,
                s.referred_bonus_vnd,
                s.referral_min_first_deposit_vnd,
                &s.quick_deposit_amounts,
                Some(&s.smm_api_base_url),
                Some(&s.smm_api_key),
                Some(new_auto_push),
                Some(&s.smm_default_mode),
                Some(new_auto_refund),
            )
            .await;
            send_settings_panel(&admin_bot, chat_id, &state).await?;
        }
        return Ok(());
    }

    Ok(())
}

async fn send_dashboard(admin_bot: &Bot, chat_id: ChatId, state: &SharedState) -> ResponseResult<()> {
    let summary = user_repo::get_users_summary(&state.pool).await.ok();
    let (_, pending_count) = state
        .activation_service
        .list_orders_for_admin(Some("PENDING"), 1, 1)
        .await
        .unwrap_or_default();
    let (_, processing_count) = state
        .activation_service
        .list_orders_for_admin(Some("PROCESSING"), 1, 1)
        .await
        .unwrap_or_default();
    let settings = settings_repo::get_settings(&state.pool).await.ok();

    let api_bal_line = if let Some(s) = settings.as_ref().filter(|s| !s.smm_api_key.trim().is_empty()) {
        match smm_service::fetch_partner_info(&s.smm_api_base_url, &s.smm_api_key).await {
            Ok(info) => {
                let coins = info.get("coins").and_then(|v| v.as_i64()).unwrap_or(0);
                let usd = (coins as f64) * 0.04;
                format!("<b>{coins} xu</b> (${usd:.2})")
            }
            Err(_) => "⚠️ Không kết nối được".to_string(),
        }
    } else {
        "<i>Chưa cấu hình API Key</i>".to_string()
    };

    let auto_push_txt = settings
        .as_ref()
        .map(|s| if s.smm_auto_push { "🟢 BẬT (tự đẩy)" } else { "🟡 TẮT (duyệt tay)" })
        .unwrap_or("--");

    let (total_users, total_dep, total_bal, banned) = summary
        .map(|s| {
            (
                s.total_users,
                s.total_deposited_vnd,
                s.total_balance_vnd,
                s.banned_users,
            )
        })
        .unwrap_or((0, 0, 0, 0));

    let text = format!(
        "📊 <b>TỔNG QUAN HỆ THỐNG GEMINI PRO</b>\n\
         ══════════════════════════\n\n\
         📥 Đơn chờ duyệt (PENDING): <b>{pending_count}</b>\n\
         ⚡ Đơn đang chạy API: <b>{processing_count}</b>\n\
         🚀 Chế độ đẩy đơn API: <b>{auto_push_txt}</b>\n\
         🪙 Số dư API đối tác: {api_bal_line}\n\n\
         👥 Tổng người dùng: <b>{total_users}</b> (Bị khóa: {banned})\n\
         💳 Tổng tiền đã nạp: <b>{} đ</b>\n\
         💰 Tổng số dư ví khách: <b>{} đ</b>",
        format_vnd(total_dep),
        format_vnd(total_bal),
    );

    let mut rows = vec![vec![
        InlineKeyboardButton::callback("🔄 Làm mới", "adm:dash"),
        InlineKeyboardButton::callback(format!("📥 Xem đơn chờ ({pending_count})"), "adm:orders"),
    ]];
    if let Ok(url) = Url::parse(&format!("{}/admin", state.config.public_base_url)) {
        rows.push(vec![InlineKeyboardButton::url("🌐 Mở Web Admin", url)]);
    }

    admin_bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;
    Ok(())
}

async fn send_pending_orders(
    admin_bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let (orders, total) = match state
        .activation_service
        .list_orders_for_admin(Some("PENDING"), 1, 10)
        .await
    {
        Ok(res) => res,
        Err(err) => {
            admin_bot
                .send_message(chat_id, format!("❌ Lỗi tải đơn hàng: {err}"))
                .await?;
            return Ok(());
        }
    };

    if orders.is_empty() {
        admin_bot
            .send_message(chat_id, "✅ Hiện không có đơn nào đang chờ xử lý (PENDING).")
            .await?;
        return Ok(());
    }

    admin_bot
        .send_message(
            chat_id,
            format!("📥 <b>DANH SÁCH ĐƠN CHỜ XỬ LÝ ({total} đơn)</b> — hiển thị tối đa 10 đơn:"),
        )
        .parse_mode(ParseMode::Html)
        .await?;

    for o in orders {
        let card = format!(
            "🧾 <b>#{}</b> • <b>{} đ</b>\n\
             • Gói: <b>{}</b> (<code>{}</code>)\n\
             • Thông tin (chạm để copy):\n\
             <code>{}</code>",
            o.order_code,
            format_vnd(o.price_paid_vnd),
            escape_html(&o.plan_name),
            o.plan_api_mode,
            escape_html(&o.credentials_formatted),
        );
        let kb = InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                format!("🚀 Đẩy qua API ({})", o.plan_api_mode),
                format!("adm:push:{}", o.id),
            )],
            vec![
                InlineKeyboardButton::callback("✅ Hoàn tất", format!("adm:done:{}", o.id)),
                InlineKeyboardButton::callback("↩️ Hoàn tiền 100%", format!("adm:ref:{}", o.id)),
            ],
        ]);
        admin_bot
            .send_message(chat_id, card)
            .parse_mode(ParseMode::Html)
            .reply_markup(kb)
            .await?;
    }
    Ok(())
}

async fn send_plans_manager(
    admin_bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let Ok(plans) = plan_repo::list_all_plans(&state.pool).await else {
        admin_bot
            .send_message(chat_id, "❌ Không thể tải danh sách gói.")
            .await?;
        return Ok(());
    };

    if plans.is_empty() {
        admin_bot
            .send_message(chat_id, "Chưa có gói dịch vụ nào.")
            .await?;
        return Ok(());
    }

    for p in plans {
        let status = if !p.is_active {
            "⛔ Đang tạm ẩn"
        } else if p.is_maintenance {
            "🔧 Đang bảo trì"
        } else {
            "🟢 Đang mở bán"
        };
        let card = format!(
            "📦 <b>{}</b> (<code>{}</code>)\n\
             • Trạng thái: <b>{}</b>\n\
             • Giá: <b>{} đ</b> • Mode: <code>{}</code>\n\
             • Tồn kho: <b>{}</b> • Đã bán: <b>{}</b>",
            escape_html(&p.name),
            escape_html(&p.code),
            status,
            format_vnd(p.price_vnd),
            p.api_mode,
            p.available_stock,
            p.sold_count,
        );
        let maint_btn = if p.is_maintenance {
            InlineKeyboardButton::callback("✅ Tắt bảo trì & báo khách", format!("adm:maint:{}", p.id))
        } else {
            InlineKeyboardButton::callback("🔧 Bật bảo trì & báo khách", format!("adm:maint:{}", p.id))
        };
        let active_btn = if p.is_active {
            InlineKeyboardButton::callback("👁️ Tạm ẩn", format!("adm:active:{}", p.id))
        } else {
            InlineKeyboardButton::callback("🟢 Mở bán", format!("adm:active:{}", p.id))
        };
        let kb = InlineKeyboardMarkup::new(vec![
            vec![maint_btn],
            vec![
                InlineKeyboardButton::callback("➕ Cộng tồn kho", format!("adm:stock:{}", p.id)),
                active_btn,
            ],
        ]);
        admin_bot
            .send_message(chat_id, card)
            .parse_mode(ParseMode::Html)
            .reply_markup(kb)
            .await?;
    }
    Ok(())
}

async fn send_broadcast_menu(admin_bot: &Bot, chat_id: ChatId) -> ResponseResult<()> {
    let text = "📢 <b>GỬI THÔNG BÁO TỚI TẤT CẢ KHÁCH HÀNG</b>\n\
                ══════════════════════════\n\n\
                Chọn mẫu thông báo nhanh hoặc tự soạn nội dung để gửi tới toàn bộ khách hàng đang hoạt động qua Bot chính:";
    let kb = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "🔧 Gửi thông báo bảo trì hệ thống",
            "adm:bc:maint",
        )],
        vec![InlineKeyboardButton::callback(
            "✅ Gửi thông báo bảo trì xong",
            "adm:bc:done",
        )],
        vec![InlineKeyboardButton::callback(
            "✏️ Soạn thông báo tùy chỉnh",
            "adm:bc:custom",
        )],
    ]);
    admin_bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(kb)
        .await?;
    Ok(())
}

async fn send_settings_panel(
    admin_bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let Ok(s) = settings_repo::get_settings(&state.pool).await else {
        admin_bot
            .send_message(chat_id, "❌ Không tải được cấu hình hệ thống.")
            .await?;
        return Ok(());
    };

    let text = format!(
        "⚙️ <b>CẤU HÌNH HỆ THỐNG &amp; API</b>\n\
         ══════════════════════════\n\n\
         • Hỗ trợ: <b>{}</b>\n\
         • Nạp tối thiểu: <b>{} đ</b>\n\
         • Thưởng mời bạn: <b>{} đ</b> / Bạn mới: <b>{} đ</b>\n\
         • Tự động đẩy đơn API: <b>{}</b>\n\
         • Tự động hoàn tiền khi API lỗi: <b>{}</b>\n\n\
         👇 Bấm nút bên dưới để bật/tắt nhanh ngay lập tức:",
        escape_html(&s.support_handle),
        format_vnd(s.min_deposit_vnd),
        format_vnd(s.referral_reward_vnd),
        format_vnd(s.referred_bonus_vnd),
        if s.smm_auto_push { "🟢 BẬT" } else { "🟡 TẮT (duyệt tay)" },
        if s.smm_auto_refund_on_fail { "🟢 BẬT" } else { "🟡 TẮT" },
    );

    let mut rows = vec![
        vec![InlineKeyboardButton::callback(
            if s.smm_auto_push {
                "🟡 Tắt tự đẩy đơn API (chuyển duyệt tay)"
            } else {
                "🟢 Bật tự đẩy đơn API (hàng đợi FIFO)"
            },
            "adm:cfg:autopush",
        )],
        vec![InlineKeyboardButton::callback(
            if s.smm_auto_refund_on_fail {
                "🟡 Tắt tự hoàn tiền khi API lỗi"
            } else {
                "🟢 Bật tự hoàn tiền khi API lỗi"
            },
            "adm:cfg:autorefund",
        )],
    ];
    if let Ok(url) = Url::parse(&format!("{}/admin", state.config.public_base_url)) {
        rows.push(vec![InlineKeyboardButton::url("🌐 Mở Web Admin đầy đủ", url)]);
    }

    admin_bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;
    Ok(())
}

fn spawn_broadcast_with_report(
    state: SharedState,
    admin_bot: Bot,
    admin_chat_id: ChatId,
    text: String,
    kb: InlineKeyboardMarkup,
) {
    tokio::spawn(async move {
        let Ok(tg_ids) = user_repo::list_all_active_telegram_ids(&state.pool).await else {
            let _ = admin_bot
                .send_message(admin_chat_id, "❌ Không thể lấy danh sách người dùng từ DB.")
                .await;
            return;
        };
        let total = tg_ids.len();
        let mut sent_ok = 0usize;
        for tg_id in tg_ids {
            if state
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
        let report = format!(
            "📢 <b>HOÀN TẤT GỬI THÔNG BÁO HÀNG LOẠT</b>\n\n\
             • Đã gửi thành công: <b>{sent_ok} / {total}</b> khách hàng\n\n\
             📩 <b>Nội dung đã gửi:</b>\n\
             <blockquote>{text}</blockquote>"
        );
        let _ = admin_bot
            .send_message(admin_chat_id, report)
            .parse_mode(ParseMode::Html)
            .await;
    });
}
