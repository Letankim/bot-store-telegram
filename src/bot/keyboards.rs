use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup, WebAppInfo,
};
use url::Url;

use crate::domain::{
    gemini_plan::GeminiPlan, partner_shop::PartnerProduct, wallet::format_vnd,
};

pub const BTN_PRODUCTS: &str = "🛍️ Sản phẩm";
pub const BTN_GEMINI_PRO: &str = "🤖 Kích hoạt Gemini Pro";
pub const BTN_PARTNER_SHOP: &str = "🛍️ Cửa hàng tài khoản & Key";
pub const BTN_WALLET: &str = "💰 Ví & nạp tiền";
pub const BTN_ORDERS: &str = "📦 Đơn hàng của tôi";
pub const BTN_GUIDE: &str = "📘 Hướng dẫn 2FA & HSTT";
pub const BTN_REFERRAL: &str = "🎁 Mời bạn bè & thưởng";

/// Persistent Reply Keyboard at bottom of Telegram chat.
pub fn main_reply_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![
        vec![
            KeyboardButton::new(BTN_PRODUCTS),
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

/// Unified inline keyboard listing ALL store products (both local plans & synced account/key products).
pub fn unified_products_keyboard(
    plans: &[GeminiPlan],
    partner_products: &[PartnerProduct],
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = plans
        .iter()
        .map(|p| {
            let label = if p.is_maintenance {
                format!("🔧 {} (Bảo trì) — {}đ", p.name, format_vnd(p.price_vnd))
            } else if p.available_stock <= 0 {
                format!("🔹 {} — {}đ (Hết hàng)", p.name, format_vnd(p.price_vnd))
            } else {
                format!("🔹 {} — {}đ [Còn {}]", p.name, format_vnd(p.price_vnd), p.available_stock)
            };
            vec![InlineKeyboardButton::callback(label, format!("plan:{}", p.id))]
        })
        .collect();

    for p in partner_products {
        let icon = if p.is_maintenance {
            "🔧"
        } else if !p.emoji.is_empty() {
            p.emoji.as_str()
        } else {
            "🔹"
        };
        let label = if p.is_maintenance {
            format!("{icon} {} (Bảo trì) — {}đ", p.name, format_vnd(p.sell_price_vnd))
        } else if p.stock <= 0 {
            format!("{icon} {} — {}đ (Hết hàng)", p.name, format_vnd(p.sell_price_vnd))
        } else {
            format!("{icon} {} — {}đ [Còn {}]", p.name, format_vnd(p.sell_price_vnd), p.stock)
        };
        rows.push(vec![InlineKeyboardButton::callback(
            label,
            format!("pprod:{}", p.id),
        )]);
    }

    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại menu chính",
        "back:home",
    )]);
    InlineKeyboardMarkup::new(rows)
}

/// Inline keyboard attached under the product detail card.
pub fn plan_detail_keyboard(plan_id: &str, guide_url: &str, form_mode: &str) -> InlineKeyboardMarkup {
    let cta_label = match form_mode {
        "none" => "🛒 Kiểm tra số dư & mua ngay",
        "email_only" => "📧 Kiểm tra số dư & nhập Email mua ngay",
        "custom_note" => "📝 Kiểm tra số dư & điền thông tin mua ngay",
        _ => "🚀 Kiểm tra số dư & kích hoạt ngay",
    };
    let mut rows = vec![vec![InlineKeyboardButton::callback(
        cta_label,
        format!("chk_act:{plan_id}"),
    )]];

    if form_mode == "mail_pass_2fa" {
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
    }

    rows.push(vec![
        InlineKeyboardButton::callback("⏪ Chọn sản phẩm khác", "back:plans"),
        InlineKeyboardButton::callback("⬅️ Menu chính", "back:home"),
    ]);

    InlineKeyboardMarkup::new(rows)
}

/// Inline keyboard shown after confirming wallet balance is sufficient -> opens Mini App Form!
pub fn open_activate_webapp_keyboard(
    activate_url: &str,
    guide_url: &str,
    form_mode: &str,
) -> InlineKeyboardMarkup {
    let form_btn_label = match form_mode {
        "email_only" => "📧 Mở form nhập Email",
        "custom_note" => "📝 Mở form nhập thông tin",
        _ => "📝 Mở form điền Mail | Pass | 2FA",
    };
    let mut rows = Vec::new();
    if let Ok(url) = Url::parse(activate_url) {
        if url.scheme() == "https" {
            rows.push(vec![InlineKeyboardButton::web_app(
                form_btn_label,
                WebAppInfo { url },
            )]);
        } else {
            rows.push(vec![InlineKeyboardButton::url(form_btn_label, url)]);
        }
    }
    if form_mode == "mail_pass_2fa" {
        if let Ok(url) = Url::parse(guide_url) {
            if url.scheme() == "https" {
                rows.push(vec![InlineKeyboardButton::web_app(
                    "📘 Xem hướng dẫn 7 bước (đóng HSTT & 2FA)",
                    WebAppInfo { url },
                )]);
            } else {
                rows.push(vec![InlineKeyboardButton::url(
                    "📘 Xem hướng dẫn 7 bước (đóng HSTT & 2FA)",
                    url,
                )]);
            }
        }
    }
    rows.push(vec![
        InlineKeyboardButton::callback("⏪ Chọn gói khác", "back:plans"),
        InlineKeyboardButton::callback("⬅️ Menu chính", "back:home"),
    ]);
    InlineKeyboardMarkup::new(rows)
}

/// Inline keyboard for `form_mode == "none"` direct confirmation without opening any form.
pub fn direct_buy_confirm_keyboard(plan_id: &str, price_vnd: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            format!("✅ Xác nhận thanh toán ngay ({} đ)", format_vnd(price_vnd)),
            format!("buy_direct:{plan_id}"),
        )],
        vec![
            InlineKeyboardButton::callback("⏪ Chọn gói khác", "back:plans"),
            InlineKeyboardButton::callback("⬅️ Menu chính", "back:home"),
        ],
    ])
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

/// Inline keyboard attached to maintenance broadcast alerts.
pub fn maintenance_broadcast_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("📋 Xem sản phẩm khác", "back:plans"),
        InlineKeyboardButton::callback("⬅️ Menu chính", "back:home"),
    ]])
}

/// Inline keyboard listing synced Partner Shop products (accounts & keys).
pub fn partner_shop_list_keyboard(products: &[PartnerProduct]) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = products
        .iter()
        .map(|p| {
            let icon = if p.is_maintenance {
                "🔧"
            } else if !p.emoji.is_empty() {
                p.emoji.as_str()
            } else {
                "🛍️"
            };
            let status_tag = if p.is_maintenance {
                " (Bảo trì)".to_string()
            } else if p.stock <= 0 {
                " (Hết hàng)".to_string()
            } else {
                format!(" [Còn {}]", p.stock)
            };
            vec![InlineKeyboardButton::callback(
                format!("{icon} {} — {}đ{status_tag}", p.name, format_vnd(p.sell_price_vnd)),
                format!("pprod:{}", p.id),
            )]
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại menu chính",
        "back:home",
    )]);
    InlineKeyboardMarkup::new(rows)
}

/// Inline keyboard with instant-buy quantity buttons under a Partner Shop product card.
pub fn partner_product_detail_keyboard(product: &PartnerProduct) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    let pid = product.id;

    let mut buy_row1 = vec![InlineKeyboardButton::callback(
        format!("🛒 Mua 1 ({}đ)", format_vnd(product.sell_price_vnd)),
        format!("pbuy:{pid}:1"),
    )];
    if product.stock >= 2 {
        buy_row1.push(InlineKeyboardButton::callback(
            format!("🛒 Mua 2 ({}đ)", format_vnd(product.sell_price_vnd * 2)),
            format!("pbuy:{pid}:2"),
        ));
    }
    rows.push(buy_row1);

    let mut buy_row2 = Vec::new();
    if product.stock >= 3 {
        buy_row2.push(InlineKeyboardButton::callback(
            format!("🛒 Mua 3 ({}đ)", format_vnd(product.sell_price_vnd * 3)),
            format!("pbuy:{pid}:3"),
        ));
    }
    if product.stock >= 5 {
        buy_row2.push(InlineKeyboardButton::callback(
            format!("🛒 Mua 5 ({}đ)", format_vnd(product.sell_price_vnd * 5)),
            format!("pbuy:{pid}:5"),
        ));
    }
    if !buy_row2.is_empty() {
        rows.push(buy_row2);
    }

    rows.push(vec![
        InlineKeyboardButton::callback("⏪ Sản phẩm khác", "back:plans"),
        InlineKeyboardButton::callback("⬅️ Menu chính", "back:home"),
    ]);

    InlineKeyboardMarkup::new(rows)
}
