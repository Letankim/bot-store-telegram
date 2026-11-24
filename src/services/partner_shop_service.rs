use std::{collections::HashSet, time::Duration};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};
use uuid::Uuid;

use crate::{
    bot::messages::escape_html,
    domain::{
        partner_shop::{PartnerOrder, PartnerProduct},
        user::User,
        wallet::format_vnd,
    },
    error::AppError,
    repo::{partner_repo, settings_repo, user_repo, wallet_repo},
    state::SharedState,
};

#[derive(Debug, Clone, Serialize)]
pub struct PartnerBalanceInfo {
    pub balance: i64,
    pub currency: String,
    pub api_wallet: i64,
    pub telegram_wallet: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct UpstreamCategory {
    name: Option<String>,
    emoji: Option<String>,
    sort: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
struct UpstreamProduct {
    id: i64,
    name: String,
    #[serde(default)]
    description: Option<String>,
    price: serde_json::Value,
    #[serde(default)]
    stock: i32,
    #[serde(rename = "type", default)]
    product_type: Option<String>,
    #[serde(default)]
    emoji: Option<String>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    category: Option<UpstreamCategory>,
}

#[derive(Debug, Deserialize)]
struct UpstreamProductsResp {
    success: Option<bool>,
    #[serde(default)]
    products: Vec<UpstreamProduct>,
    error: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct UpstreamBuyReq {
    product_id: i64,
    quantity: i32,
}

#[derive(Debug, Clone)]
pub struct PartnerBuyOutcome {
    pub items: Vec<String>,
    pub new_partner_balance: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartnerSyncSummary {
    pub total_synced: usize,
    pub new_count: usize,
    pub price_changed_count: usize,
    pub restocked_count: usize,
    pub notifications_queued: usize,
}

#[derive(Debug, Clone)]
pub struct PartnerPurchaseOutcome {
    pub order: PartnerOrder,
    pub items: Vec<String>,
    pub remaining_balance_vnd: i64,
}

fn json_val_to_i64(v: &serde_json::Value) -> i64 {
    if let Some(i) = v.as_i64() {
        return i.max(0);
    }
    if let Some(f) = v.as_f64() {
        return (f.round() as i64).max(0);
    }
    if let Some(s) = v.as_str() {
        if let Ok(i) = s.trim().parse::<i64>() {
            return i.max(0);
        }
        if let Ok(f) = s.trim().parse::<f64>() {
            return (f.round() as i64).max(0);
        }
    }
    0
}

fn format_upstream_item(item: &serde_json::Value) -> String {
    if let Some(s) = item.as_str() {
        return s.trim().to_string();
    }
    if let Some(obj) = item.as_object() {
        // Common single-field shapes like { "data": "..." } or { "content": "..." } or { "key": "..." }
        for k in ["data", "content", "value", "key", "account", "code", "credential"] {
            if let Some(s) = obj.get(k).and_then(|v| v.as_str()) {
                if !s.trim().is_empty() {
                    return s.trim().to_string();
                }
            }
        }
        // Fallback: join key=val pairs cleanly
        let parts: Vec<String> = obj
            .iter()
            .filter_map(|(k, v)| match v {
                serde_json::Value::String(s) if !s.trim().is_empty() => Some(format!("{k}: {}", s.trim())),
                serde_json::Value::Number(n) => Some(format!("{k}: {n}")),
                _ => None,
            })
            .collect();
        if !parts.is_empty() {
            return parts.join(" | ");
        }
    }
    item.to_string()
}

/// Fetch Partner Shop live wallet balance (`GET /balance`).
pub async fn fetch_partner_balance(
    base_url: &str,
    api_key: &str,
) -> Result<PartnerBalanceInfo, AppError> {
    if api_key.trim().is_empty() {
        return Err(AppError::Validation(
            "Chưa cấu hình API Key Shop đối tác.".to_string(),
        ));
    }
    let url = format!("{}/balance", base_url.trim_end_matches('/'));
    let resp = Client::new()
        .get(&url)
        .header("X-API-Key", api_key.trim())
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Không thể kết nối Shop đối tác: {e}")))?;

    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Phản hồi số dư Shop đối tác không hợp lệ: {e}")))?;

    if val.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let msg = val
            .get("message")
            .or_else(|| val.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("API Key Shop đối tác không hợp lệ");
        return Err(AppError::PaymentGateway(msg.to_string()));
    }

    let balance = val.get("balance").map(json_val_to_i64).unwrap_or(0);
    let currency = val
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("VND")
        .to_string();
    let api_wallet = val
        .get("wallets")
        .and_then(|w| w.get("api"))
        .map(json_val_to_i64)
        .unwrap_or(0);
    let telegram_wallet = val
        .get("wallets")
        .and_then(|w| w.get("telegram"))
        .map(json_val_to_i64)
        .unwrap_or(0);

    Ok(PartnerBalanceInfo {
        balance,
        currency,
        api_wallet,
        telegram_wallet,
    })
}

/// Call Partner Shop `POST /buy` to purchase digital goods (`account` / `key`) immediately.
pub async fn buy_partner_product_upstream(
    base_url: &str,
    api_key: &str,
    external_product_id: i64,
    quantity: i32,
) -> Result<PartnerBuyOutcome, AppError> {
    if api_key.trim().is_empty() {
        return Err(AppError::Validation(
            "Chưa cấu hình API Key Shop đối tác.".to_string(),
        ));
    }
    let url = format!("{}/buy", base_url.trim_end_matches('/'));
    let body = UpstreamBuyReq {
        product_id: external_product_id,
        quantity,
    };

    let resp = Client::new()
        .post(&url)
        .header("X-API-Key", api_key.trim())
        .json(&body)
        .timeout(Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Lỗi kết nối mua hàng từ Shop đối tác: {e}")))?;

    let status = resp.status();
    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Phản hồi mua hàng không hợp lệ ({status}): {e}")))?;

    if val.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = val
            .get("message")
            .or_else(|| val.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("Shop đối tác từ chối đơn mua (hết hàng hoặc không đủ số dư)");
        return Err(AppError::PaymentGateway(msg.to_string()));
    }

    let raw_items = val
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if raw_items.is_empty() {
        return Err(AppError::PaymentGateway(
            "Shop đối tác trả về danh sách sản phẩm rỗng.".to_string(),
        ));
    }

    let items: Vec<String> = raw_items.iter().map(format_upstream_item).collect();
    let new_partner_balance = val.get("new_balance").map(json_val_to_i64);

    Ok(PartnerBuyOutcome {
        items,
        new_partner_balance,
    })
}

/// Sync all products from Partner Shop API (`GET /products`), recalculate sell prices using
/// configured markup (default +30,000 VND or per-product custom markup), detect price/stock changes,
/// and broadcast customer notifications when enabled.
pub async fn sync_partner_products(
    state: &SharedState,
    trigger_notifications: bool,
) -> Result<PartnerSyncSummary, AppError> {
    let settings = settings_repo::get_settings(&state.pool).await?;
    if settings.partner_shop_api_key.trim().is_empty() {
        return Err(AppError::Validation(
            "Chưa cấu hình API Key Shop đối tác trong mục Shop đối tác (API).".to_string(),
        ));
    }

    let url = format!("{}/products", settings.partner_shop_base_url.trim_end_matches('/'));
    let resp = Client::new()
        .get(&url)
        .header("X-API-Key", settings.partner_shop_api_key.trim())
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Không thể tải danh sách sản phẩm đối tác: {e}")))?;

    let parsed: UpstreamProductsResp = resp
        .json()
        .await
        .map_err(|e| AppError::PaymentGateway(format!("Dữ liệu sản phẩm đối tác không hợp lệ: {e}")))?;

    if parsed.success == Some(false) {
        let msg = parsed
            .message
            .or(parsed.error)
            .unwrap_or_else(|| "API Shop đối tác báo lỗi khi tải sản phẩm".to_string());
        return Err(AppError::PaymentGateway(msg));
    }

    let had_existing = partner_repo::count_products(&state.pool).await? > 0;
    let existing_all = partner_repo::list_all_products(&state.pool).await?;

    let mut seen_external_ids: HashSet<i64> = HashSet::new();
    let mut new_count = 0usize;
    let mut price_changed_count = 0usize;
    let mut restocked_count = 0usize;
    let mut pending_broadcasts: Vec<(String, InlineKeyboardMarkup)> = Vec::new();
    let mut admin_change_lines: Vec<String> = Vec::new();

    for ext in &parsed.products {
        seen_external_ids.insert(ext.id);
        let partner_price = json_val_to_i64(&ext.price);
        let prev = existing_all.iter().find(|p| p.external_id == ext.id);

        let cat_name = ext.category.as_ref().and_then(|c| c.name.as_deref());
        let cat_emoji = ext.category.as_ref().and_then(|c| c.emoji.as_deref());
        let cat_sort = ext.category.as_ref().and_then(|c| c.sort).unwrap_or(0);

        let updated = partner_repo::upsert_synced_product(
            &state.pool,
            ext.id,
            &ext.name,
            ext.description.as_deref().unwrap_or(""),
            ext.product_type.as_deref().unwrap_or("account"),
            ext.emoji.as_deref().unwrap_or(""),
            ext.image_url.as_deref(),
            cat_name,
            cat_emoji,
            cat_sort,
            partner_price,
            settings.partner_shop_markup_vnd,
            ext.stock,
        )
        .await?;

        let can_notify = updated.is_active && !updated.is_maintenance;

        match prev {
            None => {
                new_count += 1;
                if had_existing && can_notify && updated.stock > 0 {
                    let msg = format_new_partner_product_broadcast(&updated);
                    let kb = partner_product_broadcast_kb(&updated.id.to_string());
                    pending_broadcasts.push((msg, kb));
                    admin_change_lines.push(format!(
                        "🆕 Mới: <b>{}</b> — Giá bán: <b>{}đ</b> (Kho: {})",
                        escape_html(&updated.name),
                        format_vnd(updated.sell_price_vnd),
                        updated.stock
                    ));
                }
            }
            Some(old) => {
                let price_changed = old.sell_price_vnd != updated.sell_price_vnd;
                let stock_added = updated.stock - old.stock;

                if price_changed {
                    price_changed_count += 1;
                    admin_change_lines.push(format!(
                        "{} <b>{}</b>: {}đ → <b>{}đ</b> (Gốc: {}đ → {}đ)",
                        if updated.sell_price_vnd < old.sell_price_vnd { "📉 Giảm giá" } else { "📈 Tăng giá" },
                        escape_html(&updated.name),
                        format_vnd(old.sell_price_vnd),
                        format_vnd(updated.sell_price_vnd),
                        format_vnd(old.partner_price_vnd),
                        format_vnd(updated.partner_price_vnd),
                    ));
                    if can_notify && updated.stock > 0 {
                        let msg = format_price_change_broadcast(
                            &updated,
                            old.sell_price_vnd,
                            updated.sell_price_vnd,
                        );
                        let kb = partner_product_broadcast_kb(&updated.id.to_string());
                        pending_broadcasts.push((msg, kb));
                    }
                } else if stock_added > 0 {
                    restocked_count += 1;
                    admin_change_lines.push(format!(
                        "📦 Lên kho: <b>{}</b> (+{} → tổng {})",
                        escape_html(&updated.name),
                        stock_added,
                        updated.stock
                    ));
                    if can_notify {
                        let msg = format_partner_restock_broadcast(&updated, stock_added);
                        let kb = partner_product_broadcast_kb(&updated.id.to_string());
                        pending_broadcasts.push((msg, kb));
                    }
                }
            }
        }
    }

    // Mark local products that disappeared from upstream as stock = 0
    for old in &existing_all {
        if !seen_external_ids.contains(&old.external_id) && old.stock > 0 {
            let _ = sqlx::query(
                "UPDATE partner_products SET stock = 0, last_synced_at = NOW(), updated_at = NOW() WHERE id = $1",
            )
            .bind(old.id)
            .execute(&state.pool)
            .await;
        }
    }

    let notifications_queued = if trigger_notifications && settings.partner_shop_notify_changes {
        pending_broadcasts.len()
    } else {
        0
    };

    if notifications_queued > 0 {
        let state_bg = state.clone();
        tokio::spawn(async move {
            let Ok(tg_ids) = user_repo::list_all_active_telegram_ids(&state_bg.pool).await else {
                return;
            };
            // Cap to 5 alerts per sync cycle to avoid flooding customers if partner bulk-updates
            for (text, kb) in pending_broadcasts.into_iter().take(5) {
                for &tg_id in &tg_ids {
                    let _ = state_bg
                        .bot
                        .send_message(ChatId(tg_id), &text)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(kb.clone())
                        .await;
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            }
        });
    }

    if !admin_change_lines.is_empty() {
        if let Some(admin_id) = state.config.admin_telegram_id {
            let summary_msg = format!(
                "🔄 <b>[SHOP ĐỐI TÁC] CẬP NHẬT TỰ ĐỘNG</b>\n\
                 ━━━━━━━━━━━━━━━━━━━━\n\
                 {}\n\n\
                 📢 Đã gửi thông báo khách: <b>{}</b>",
                admin_change_lines.into_iter().take(15).collect::<Vec<_>>().join("\n"),
                if notifications_queued > 0 { "Có" } else { "Không" }
            );
            let _ = state
                .bot
                .send_message(ChatId(admin_id), summary_msg)
                .parse_mode(ParseMode::Html)
                .await;
        }
    }

    Ok(PartnerSyncSummary {
        total_synced: parsed.products.len(),
        new_count,
        price_changed_count,
        restocked_count,
        notifications_queued,
    })
}

/// Spawn background Tokio task that periodically syncs products, prices & stock from Partner Shop API.
pub fn spawn_partner_shop_sync_worker(state: SharedState) {
    tokio::spawn(async move {
        // Short initial delay so HTTP server boots cleanly first
        tokio::time::sleep(Duration::from_secs(8)).await;
        loop {
            let interval_secs = match settings_repo::get_settings(&state.pool).await {
                Ok(settings) => {
                    if settings.partner_shop_enabled && !settings.partner_shop_api_key.trim().is_empty() {
                        match sync_partner_products(&state, true).await {
                            Ok(sum) => {
                                tracing::info!(
                                    total = sum.total_synced,
                                    new = sum.new_count,
                                    price_changed = sum.price_changed_count,
                                    restocked = sum.restocked_count,
                                    "Partner Shop periodic sync completed"
                                );
                            }
                            Err(err) => {
                                tracing::warn!(error = ?err, "Partner Shop periodic sync failed");
                            }
                        }
                    }
                    settings.partner_shop_sync_interval_secs.clamp(30, 86_400) as u64
                }
                Err(_) => 180,
            };
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;
        }
    });
}

/// Execute an instant Partner Shop purchase:
/// 1. ACID TX: reserve stock + debit customer wallet + create order (`PROCESSING`)
/// 2. Call upstream `POST /api/partner-api/buy`
/// 3. On success: encrypt items with AES-256-GCM, mark `COMPLETED`, notify Admin with profit breakdown
/// 4. On failure: ACID refund 100% to customer wallet + restore stock + alert Admin urgently
pub async fn purchase_partner_product(
    state: &SharedState,
    user: User,
    product_id: Uuid,
    quantity: i32,
) -> Result<PartnerPurchaseOutcome, AppError> {
    if user.is_banned {
        return Err(AppError::Forbidden(
            "Tài khoản của bạn đã bị khóa, không thể mua hàng.".to_string(),
        ));
    }
    if !(1..=20).contains(&quantity) {
        return Err(AppError::Validation(
            "Số lượng mua mỗi lần từ 1 đến 20 sản phẩm.".to_string(),
        ));
    }

    let settings = settings_repo::get_settings(&state.pool).await?;
    if settings.partner_shop_api_key.trim().is_empty() {
        return Err(AppError::PaymentGateway(
            "Hệ thống cửa hàng tài khoản tạm thời chưa sẵn sàng.".to_string(),
        ));
    }

    // Phase 1: ACID wallet debit + stock reservation
    let mut tx = state.pool.begin().await?;
    let product = partner_repo::reserve_stock_tx(&mut tx, product_id, quantity).await?;
    let total_sell_price = product.sell_price_vnd.saturating_mul(i64::from(quantity));
    if total_sell_price <= 0 {
        return Err(AppError::Validation("Giá sản phẩm không hợp lệ.".to_string()));
    }

    let wallet = wallet_repo::lock_by_user_id(&mut tx, user.id).await?;
    let order_code = format!(
        "SP{}",
        &Uuid::new_v4().simple().to_string()[..8].to_uppercase()
    );
    let idem_key = format!("partner_buy_{order_code}");
    let desc = format!("Mua {} x{}", product.name, quantity);

    let updated_wallet = wallet_repo::debit_wallet(
        &mut tx,
        &wallet,
        total_sell_price,
        "PARTNER_SHOP_PURCHASE",
        &idem_key,
        &desc,
    )
    .await?;

    let order = partner_repo::create_order_tx(&mut tx, &order_code, user.id, &product, quantity).await?;
    tx.commit().await?;

    // Phase 2: Call Partner Shop API `POST /buy`
    match buy_partner_product_upstream(
        &settings.partner_shop_base_url,
        &settings.partner_shop_api_key,
        product.external_id,
        quantity,
    )
    .await
    {
        Ok(buy_res) => {
            let items_json = serde_json::to_string(&buy_res.items)
                .unwrap_or_else(|_| "[]".to_string());
            let (ciphertext, nonce) = state.crypto.encrypt(&items_json)?;

            let completed = partner_repo::mark_order_completed(
                &state.pool,
                order.id,
                product.id,
                quantity,
                &ciphertext,
                &nonce,
            )
            .await?;

            // Notify Admin with revenue, cost & net profit
            if let Some(admin_id) = state.config.admin_telegram_id {
                let partner_bal_line = buy_res
                    .new_partner_balance
                    .map(|b| format!("\n• Số dư ví Shop đối tác còn: <b>{} đ</b>", format_vnd(b)))
                    .unwrap_or_default();
                let cust_ident = if user.telegram_id > 0 {
                    format!("{} (TG ID: <code>{}</code>)", escape_html(&user.full_name), user.telegram_id)
                } else {
                    format!("{} (Web: <code>{}</code>)", escape_html(&user.full_name), escape_html(user.email.as_deref().unwrap_or("web")))
                };
                let admin_msg = format!(
                    "🛍️ <b>ĐƠN SHOP ĐỐI TÁC THÀNH CÔNG!</b>\n\n\
                     • Mã đơn: <code>{}</code>\n\
                     • Sản phẩm: <b>{}</b> x{}\n\
                     • Khách hàng: {}\n\
                     • Thu khách: <b>{} đ</b>\n\
                     • Giá gốc đối tác: <b>{} đ</b>\n\
                     • Lợi nhuận ròng: <b>+{} đ</b>{}",
                    completed.order_code,
                    escape_html(&completed.product_name),
                    completed.quantity,
                    cust_ident,
                    format_vnd(completed.total_sell_price_vnd),
                    format_vnd(completed.total_partner_cost_vnd),
                    format_vnd(completed.profit_vnd),
                    partner_bal_line,
                );
                let _ = state
                    .bot
                    .send_message(ChatId(admin_id), admin_msg)
                    .parse_mode(ParseMode::Html)
                    .await;
            }

            Ok(PartnerPurchaseOutcome {
                order: completed,
                items: buy_res.items,
                remaining_balance_vnd: updated_wallet.balance_vnd,
            })
        }
        Err(upstream_err) => {
            let err_str = upstream_err.to_string();
            tracing::error!(
                order_code = %order.order_code,
                error = %err_str,
                "Upstream Partner Shop buy failed; executing automatic 100% refund"
            );

            // Phase 3 (Error path): ACID 100% refund to customer wallet + restore local stock
            let mut rtx = state.pool.begin().await?;
            let w_lock = wallet_repo::lock_by_user_id(&mut rtx, user.id).await?;
            let refund_idem = format!("partner_refund_{}", order.id);
            let refund_desc = format!("Hoàn 100% tiền đơn {} (tạm hết hàng)", order.order_code);
            let _ = wallet_repo::credit_wallet(
                &mut rtx,
                &w_lock,
                total_sell_price,
                false,
                "ORDER_REFUND",
                &refund_idem,
                &refund_desc,
            )
            .await?;
            let _ = partner_repo::mark_order_refunded_tx(
                &mut rtx,
                order.id,
                product.id,
                quantity,
                &err_str,
            )
            .await?;
            rtx.commit().await?;

            // Urgent Admin notification so Admin can top up partner wallet or check stock
            if let Some(admin_id) = state.config.admin_telegram_id {
                let alert = format!(
                    "🚨 <b>[CẢNH BÁO] LỖI MUA HÀNG SHOP ĐỐI TÁC!</b>\n\n\
                     • Mã đơn: <code>{}</code>\n\
                     • Sản phẩm: <b>{}</b> x{}\n\
                     • Lỗi từ đối tác: <i>{}</i>\n\n\
                     ↩️ Hệ thống đã <b>tự động hoàn 100% ({} đ)</b> về ví khách hàng.\n\
                     👉 Kiểm tra số dư ví Shop đối tác hoặc tồn kho trong <code>/admin</code>!",
                    order.order_code,
                    escape_html(&product.name),
                    quantity,
                    escape_html(&err_str),
                    format_vnd(total_sell_price),
                );
                let _ = state
                    .bot
                    .send_message(ChatId(admin_id), alert)
                    .parse_mode(ParseMode::Html)
                    .await;
            }

            Err(AppError::PaymentGateway(
                "Sản phẩm vừa tạm hết hàng hoặc hệ thống cung cấp đang bận. Đã tự động hoàn 100% tiền về ví của bạn!".to_string(),
            ))
        }
    }
}

/// Decrypt delivered items for a completed PartnerOrder.
pub fn decrypt_order_items(state: &SharedState, order: &PartnerOrder) -> Vec<String> {
    let (Some(ct), Some(nonce)) = (order.items_encrypted.as_ref(), order.nonce.as_ref()) else {
        return Vec::new();
    };
    let Ok(raw) = state.crypto.decrypt(ct, nonce) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_else(|_| vec![raw])
}

fn partner_product_broadcast_kb(product_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("🛒 Mua ngay", format!("pprod:{product_id}")),
        InlineKeyboardButton::callback("🛍️ Xem cửa hàng", "pshop:list"),
    ]])
}

fn format_partner_restock_broadcast(p: &PartnerProduct, added_qty: i32) -> String {
    let name = escape_html(&p.name);
    let prefix = if p.emoji.is_empty() { "✨" } else { &p.emoji };
    format!(
        "📢 <b>HÀNG MỚI VỪA LÊN KHO</b>\n\
         ━━━━━━━━━━━━━━━━━━━━\n\n\
         {prefix} <b>{name}</b>\n\n\
         📦 Sản phẩm: <b>{name}</b>\n\
         💵 Giá: <b>{}đ</b>\n\
         ✅ Vừa thêm: <b>{added_qty} sản phẩm</b>\n\
         📊 Kho hiện tại: <b>{} sản phẩm</b>",
        format_vnd(p.sell_price_vnd),
        p.stock,
    )
}

fn format_price_change_broadcast(p: &PartnerProduct, old_price: i64, new_price: i64) -> String {
    let name = escape_html(&p.name);
    let is_drop = new_price < old_price;
    let header = if is_drop {
        "🔥 <b>GIẢM GIÁ SẢN PHẨM!</b>"
    } else {
        "📢 <b>CẬP NHẬT GIÁ SẢN PHẨM</b>"
    };
    format!(
        "{header}\n\
         ━━━━━━━━━━━━━━━━━━━━\n\n\
         📦 Sản phẩm: <b>{name}</b>\n\
         💸 Giá cũ: <s>{}đ</s>\n\
         💵 Giá mới: <b>{}đ</b>\n\
         📊 Kho hiện tại: <b>{} sản phẩm</b>",
        format_vnd(old_price),
        format_vnd(new_price),
        p.stock,
    )
}

fn format_new_partner_product_broadcast(p: &PartnerProduct) -> String {
    let name = escape_html(&p.name);
    let prefix = if p.emoji.is_empty() { "🆕" } else { &p.emoji };
    format!(
        "🆕 <b>SẢN PHẨM MỚI VỪA LÊN KỆ</b>\n\
         ━━━━━━━━━━━━━━━━━━━━\n\n\
         {prefix} <b>{name}</b>\n\n\
         💵 Giá: <b>{}đ</b>\n\
         📊 Kho hiện tại: <b>{} sản phẩm</b>",
        format_vnd(p.sell_price_vnd),
        p.stock,
    )
}
