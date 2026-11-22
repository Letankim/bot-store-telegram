use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup, WebAppInfo,
};
use url::Url;

use crate::domain::{gemini_plan::GeminiPlan, wallet::format_vnd};

pub const BTN_GEMINI_PRO: &str = "🤖 Kích hoạt Gemini Pro";
pub const BTN_WALLET: &str = "💰 Ví & nạp tiền";
pub const BTN_ORDERS: &str = "📦 Đơn hàng của tôi";
pub const BTN_GUIDE: &str = "📘 Hướng dẫn 2FA & HSTT";
pub const BTN_REFERRAL: &str = "🎁 Mời bạn bè & thưởng";

/// Persistent Reply Keyboard at bottom of Telegram chat.
pub fn main_reply_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![
        vec![
            KeyboardButton::new(BTN_GEMINI_PRO),
            KeyboardButton::new(BTN_ORDERS),
        ],
        vec![
            KeyboardButton::new(BTN_WALLET),
            KeyboardButton::new(BTN_GUIDE),
        ],
        vec![KeyboardButton::new(BTN_REFERRAL)],
    ])
    .resize_keyboard()
    .persistent()
}

/// Inline keyboard listing active Gemini Pro plans + Back to Home.
pub fn plan_selector_keyboard(plans: &[GeminiPlan]) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = plans
        .iter()
        .map(|p| {
            let label = if p.is_maintenance {
                format!("🔧 {} (Bảo trì) — {} đ", p.name, format_vnd(p.price_vnd))
            } else {
                format!("🔹 {} — {} đ", p.name, format_vnd(p.price_vnd))
            };
            vec![InlineKeyboardButton::callback(label, format!("plan:{}", p.id))]
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại menu chính",
        "back:home",
    )]);
    InlineKeyboardMarkup::new(rows)
}

/// Inline keyboard attached under the Gemini Pro detail card.
pub fn plan_detail_keyboard(plan_id: &str, guide_url: &str) -> InlineKeyboardMarkup {
    let mut rows = vec![vec![InlineKeyboardButton::callback(
        "🚀 Kiểm tra số dư & kích hoạt ngay",
        format!("chk_act:{plan_id}"),
    )]];

    if let Ok(parsed) = Url::parse(guide_url) {
        if parsed.scheme() == "https" {
            rows.push(vec![InlineKeyboardButton::web_app(
                "📘 Xem hướng dẫn đóng HSTT & lấy 2FA",
                WebAppInfo { url: parsed },
            )]);
        } else {
            rows.push(vec![InlineKeyboardButton::url(
                "📘 Xem hướng dẫn đóng HSTT & lấy 2FA",
                parsed,
            )]);
        }
    }

    rows.push(vec![
        InlineKeyboardButton::callback("⏪ Chọn gói khác", "back:plans"),
        InlineKeyboardButton::callback("⬅️ Menu chính", "back:home"),
    ]);

    InlineKeyboardMarkup::new(rows)
}

/// Inline keyboard shown after confirming wallet balance is sufficient -> opens Mini App Form!
pub fn open_activate_webapp_keyboard(
    activate_url: &str,
    guide_url: &str,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if let Ok(url) = Url::parse(activate_url) {
        if url.scheme() == "https" {
            rows.push(vec![InlineKeyboardButton::web_app(
                "📝 Mở form điền Mail | Pass | 2FA",
                WebAppInfo { url },
            )]);
        } else {
            rows.push(vec![InlineKeyboardButton::url(
                "📝 Mở form điền Mail | Pass | 2FA",
                url,
            )]);
        }
    }
    if let Ok(url) = Url::parse(guide_url) {
        if url.scheme() == "https" {
            rows.push(vec![InlineKeyboardButton::web_app(
                "📘 Xem hướng dẫn 6 bước (đóng HSTT & 2FA)",
                WebAppInfo { url },
            )]);
        } else {
            rows.push(vec![InlineKeyboardButton::url(
                "📘 Xem hướng dẫn 6 bước (đóng HSTT & 2FA)",
                url,
            )]);
        }
    }
    rows.push(vec![
        InlineKeyboardButton::callback("⏪ Chọn gói khác", "back:plans"),
        InlineKeyboardButton::callback("⬅️ Menu chính", "back:home"),
    ]);
    InlineKeyboardMarkup::new(rows)
}

/// Inline keyboard for quick VietQR top-up amounts + Custom amount input + Back.
pub fn payos_amount_keyboard(
    suggested_missing: Option<i64>,
    min_deposit_vnd: i64,
    quick_amounts: &[i64],
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if let Some(missing) = suggested_missing {
        let rounded = missing.max(min_deposit_vnd);
        rows.push(vec![InlineKeyboardButton::callback(
            format!("⚡ Nạp nhanh đúng số tiền thiếu ({} đ)", format_vnd(rounded)),
            format!("payos_amt:{rounded}"),
        )]);
    }
    for chunk in quick_amounts.chunks(2) {
        let row = chunk
            .iter()
            .map(|&amt| {
                InlineKeyboardButton::callback(
                    format!("💳 {} đ", format_vnd(amt)),
                    format!("payos_amt:{amt}"),
                )
            })
            .collect();
        rows.push(row);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "✏️ Nhập số tiền khác",
        "payos_custom",
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại menu chính",
        "back:home",
    )]);
    InlineKeyboardMarkup::new(rows)
}

/// Inline keyboard shown directly under the VietQR photo in Telegram chat.
pub fn vietqr_payment_actions_keyboard(order_code: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("✅ Tôi đã thanh toán", format!("payos_chk:{order_code}")),
            InlineKeyboardButton::callback("❌ Hủy thanh toán", format!("payos_cancel:{order_code}")),
        ],
        vec![InlineKeyboardButton::callback(
            "⬅️ Quay lại menu chính",
            "back:home",
        )],
    ])
}

/// Simple Back-to-Home inline button.
pub fn back_to_home_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại menu chính",
        "back:home",
    )]])
}

/// Inline keyboard attached to restock broadcast alerts ("Mua ngay" + "Xem sản phẩm khác").
pub fn restock_broadcast_keyboard(plan_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("🛒 Mua ngay", format!("plan:{plan_id}")),
        InlineKeyboardButton::callback("📋 Xem sản phẩm khác", "back:plans"),
    ]])
}
