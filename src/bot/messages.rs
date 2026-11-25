use crate::domain::{
    gemini_plan::GeminiPlan, partner_shop::PartnerProduct, wallet::format_vnd,
};

/// Escape HTML special characters for Telegram `ParseMode::Html`.
pub fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn welcome_message(
    full_name: &str,
    balance_vnd: i64,
    ref_link: &str,
    reward_vnd: i64,
    referred_bonus_vnd: i64,
    min_first_deposit_vnd: i64,
    support_handle: &str,
) -> String {
    let friend_bonus_line = if referred_bonus_vnd > 0 {
        format!(
            "\n<i>• Bạn bè được mời cũng nhận ngay <b>{} đ</b> thưởng vào ví khi hoàn tất lần nạp đầu tiên!</i>",
            format_vnd(referred_bonus_vnd)
        )
    } else {
        String::new()
    };

    format!(
        "👋 Xin chào <b>{}</b>!\n\n\
         🛍️ Chào mừng bạn đến với hệ thống <b>cửa hàng sản phẩm số &amp; dịch vụ nâng cấp chính chủ</b> tự động 24/7.\n\n\
         💰 Số dư ví hiện tại: <b>{} đ</b>\n\
         🎁 <b>Chương trình mời bạn bè:</b>\n\
         <code>{}</code>\n\
         <i>• Bạn nhận ngay <b>{} đ</b> khi bạn bè đăng ký qua link và bắt buộc nạp tiền lần đầu (tối thiểu {} đ).</i>{}\n\n\
         🛟 Hỗ trợ: {}\n\
         👇 Bấm nút <b>🛍️ Sản phẩm</b> bên dưới (hoặc gõ /shop) để xem toàn bộ sản phẩm!",
        escape_html(full_name),
        format_vnd(balance_vnd),
        escape_html(ref_link),
        format_vnd(reward_vnd),
        format_vnd(min_first_deposit_vnd),
        friend_bonus_line,
        escape_html(support_handle),
    )
}

pub fn referral_dashboard_message(
    ref_link: &str,
    reward_vnd: i64,
    referred_bonus_vnd: i64,
    min_first_deposit_vnd: i64,
    summary: &crate::services::referral_service::ReferralSummary,
) -> String {
    let mut friend_list = String::new();
    if summary.recent_friends.is_empty() {
        friend_list.push_str("<i>Chưa có bạn bè nào tham gia qua link của bạn.</i>");
    } else {
        for (idx, f) in summary.recent_friends.iter().enumerate() {
            let status_txt = if f.status == "REWARDED" {
                format!("✅ Đã nhận <b>+{} đ</b>", format_vnd(f.reward_vnd))
            } else {
                "⏳ Chờ nạp lần đầu".to_string()
            };
            friend_list.push_str(&format!(
                "{}. <b>{}</b> — {}\n",
                idx + 1,
                escape_html(&f.full_name),
                status_txt
            ));
        }
    }

    let friend_bonus_txt = if referred_bonus_vnd > 0 {
        format!("\n• Người được mời nhận: <b>+{} đ</b>", format_vnd(referred_bonus_vnd))
    } else {
        String::new()
    };

    format!(
        "🎁 <b>MỜI BẠN BÈ &amp; THỐNG KÊ TIỀN THƯỞNG</b>\n\n\
         🔗 <b>Link giới thiệu của bạn (chạm để copy):</b>\n\
         <code>{}</code>\n\n\
         📌 <b>Chính sách thưởng hiện tại:</b>\n\
         • Bạn (người mời) nhận: <b>+{} đ</b>{}\n\
         • Điều kiện: Bạn bè bắt buộc nạp tiền lần đầu tối thiểu <b>{} đ</b>.\n\n\
         📊 <b>Thống kê của bạn:</b>\n\
         • 👥 Tổng số người đã mời: <b>{} người</b>\n\
         • ✅ Đã nạp lần đầu (đã thưởng): <b>{} người</b>\n\
         • ⏳ Đang chờ nạp lần đầu: <b>{} người</b>\n\
         • 💰 Tổng tiền thưởng đã nhận: <b>{} đ</b>\n\n\
         📋 <b>Danh sách bạn bè đã mời gần nhất:</b>\n{}",
        escape_html(ref_link),
        format_vnd(reward_vnd),
        friend_bonus_txt,
        format_vnd(min_first_deposit_vnd),
        summary.total_invited,
        summary.rewarded_count,
        summary.pending_count,
        format_vnd(summary.total_earned_vnd),
        friend_list.trim_end(),
    )
}

pub fn gemini_plan_card(plan: &GeminiPlan, guide_url: &str, support_handle: &str) -> String {
    let maintenance_line = if plan.is_maintenance {
        "🔧 Trạng thái: <b>Đang bảo trì</b>\n"
    } else {
        ""
    };
    let form_req_line = match plan.form_mode.as_str() {
        "email_only" => "📝 Yêu cầu khi mua: <b>Chỉ cần nhập Email</b>\n",
        "custom_note" => "📝 Yêu cầu khi mua: <b>Điền thông tin theo yêu cầu</b>\n",
        "none" => "📝 Yêu cầu khi mua: <b>Không cần điền form — thanh toán ngay</b>\n",
        _ => "📝 Yêu cầu khi mua: <b>Mail | Pass | 2FA</b>\n",
    };
    let guide_line = if plan.form_mode == "mail_pass_2fa" {
        format!(
            "\n- Hướng dẫn đóng HSTT &amp; gắn 2FA: <a href=\"{}\">Xem trang hướng dẫn chi tiết tại đây</a>",
            escape_html(guide_url)
        )
    } else {
        String::new()
    };

    format!(
        "🔹 <b>{}</b>\n\
         {}{}💸 Giá: <b>{} đ</b>\n\
         📦 Tồn kho: <b>{} tài khoản</b>\n\
         📊 Đã bán: <b>{} tài khoản</b>\n\n\
         <blockquote>✏️ <b>Mô tả:</b>\n\
         {}{}</blockquote>\n\n\
         ‼️ Vui lòng đọc kỹ tiêu đề sản phẩm &amp; mô tả trước khi mua hàng.\n\
         Chỉ hỗ trợ refund với những đơn hàng lỗi.\n\
         Hỗ trợ: {}",
        escape_html(&plan.name),
        maintenance_line,
        form_req_line,
        format_vnd(plan.price_vnd),
        plan.available_stock,
        plan.sold_count,
        escape_html(&plan.description_note),
        guide_line,
        escape_html(support_handle)
    )
}

pub fn plan_maintenance_message(plan: &GeminiPlan) -> String {
    let note = if plan.maintenance_note.trim().is_empty() {
        "Vui lòng thử lại sau."
    } else {
        plan.maintenance_note.trim()
    };
    format!(
        "🔧 <b>{} đang bảo trì</b>\n\
         ══════════════════════════\n\n\
         {}",
        escape_html(&plan.name),
        escape_html(note),
    )
}

pub fn referral_joined_notification(
    new_user_name: &str,
    reward_vnd: i64,
    min_first_deposit_vnd: i64,
) -> String {
    format!(
        "🎁 <b>Có người mới dùng mã giới thiệu của bạn!</b>\n\
         ==================================\n\
         👤 <b>{}</b> đã tham gia qua link của bạn.\n\
         ==================================\n\
         ⏳ Bạn sẽ nhận <b>{} đ</b> thưởng khi họ hoàn tất lần nạp tiền đầu tiên (tối thiểu {} đ).",
        escape_html(new_user_name),
        format_vnd(reward_vnd),
        format_vnd(min_first_deposit_vnd)
    )
}

pub fn vietqr_photo_caption(
    order_code: i64,
    amount_vnd: i64,
    description: &str,
    account_number: Option<&str>,
    account_name: Option<&str>,
) -> String {
    let mut bank_lines = String::new();
    if let Some(acc) = account_number.filter(|s| !s.is_empty()) {
        bank_lines.push_str(&format!("\n• Số tài khoản: <code>{}</code>", escape_html(acc)));
    }
    if let Some(name) = account_name.filter(|s| !s.is_empty()) {
        bank_lines.push_str(&format!("\n• Chủ tài khoản: <b>{}</b>", escape_html(name)));
    }

    format!(
        "💳 <b>MÃ QR CHUYỂN KHOẢN TỰ ĐỘNG (VIETQR)</b>\n\n\
         • Mã giao dịch: <code>{}</code>\n\
         • Số tiền: <b>{} đ</b>\n\
         • Nội dung CK: <code>{}</code>{}\n\n\
         📱 Mở ứng dụng Ngân hàng và <b>quét trực tiếp mã QR phía trên</b>.\n\
         ⚡ Sau khi chuyển khoản, tiền sẽ tự động cộng vào ví sau <b>5–30 giây</b> (hoặc bấm <b>✅ Tôi đã thanh toán</b> để kiểm tra ngay)!",
        order_code,
        format_vnd(amount_vnd),
        escape_html(description),
        bank_lines
    )
}

pub fn restock_broadcast_message(plan: &GeminiPlan, added_qty: i32) -> String {
    let name = escape_html(&plan.name);
    format!(
        "📢 <b>HÀNG MỚI VỪA LÊN KHO</b>\n\
         ━━━━━━━━━━━━━━━━━━━━\n\n\
         ✨ <b>{name}</b>\n\n\
         📦 Sản phẩm: <b>{name}</b>\n\
         💵 Giá: <b>{}đ</b>\n\
         ✅ Vừa thêm: <b>{} tài khoản</b>\n\
         📊 Kho hiện tại: <b>{} tài khoản</b>",
        format_vnd(plan.price_vnd),
        added_qty,
        plan.available_stock,
    )
}

pub fn maintenance_started_broadcast_message(plan: &GeminiPlan) -> String {
    let name = escape_html(&plan.name);
    let note = if plan.maintenance_note.trim().is_empty() {
        "Vui lòng thử lại sau."
    } else {
        plan.maintenance_note.trim()
    };
    format!(
        "🔧 <b>THÔNG BÁO BẢO TRÌ DỊCH VỤ</b>\n\
         ══════════════════════════\n\n\
         📦 Sản phẩm: <b>{name}</b>\n\
         📝 Nội dung: {}\n\n\
         ⏳ Hệ thống sẽ thông báo ngay khi dịch vụ hoạt động trở lại. Cảm ơn bạn đã thông cảm!",
        escape_html(note),
    )
}

pub fn maintenance_ended_broadcast_message(plan: &GeminiPlan) -> String {
    let name = escape_html(&plan.name);
    format!(
        "✅ <b>DỊCH VỤ ĐÃ BẢO TRÌ XONG &amp; MỞ BÁN LẠI</b>\n\
         ══════════════════════════\n\n\
         📦 Sản phẩm: <b>{name}</b>\n\
         💵 Giá: <b>{}đ</b>\n\
         📊 Kho hiện tại: <b>{} tài khoản</b>\n\n\
         🚀 Bạn đã có thể đặt đơn ngay bây giờ!",
        format_vnd(plan.price_vnd),
        plan.available_stock,
    )
}

pub fn partner_product_card(product: &PartnerProduct, support_handle: &str) -> String {
    let icon = if product.emoji.is_empty() { "🛍️" } else { &product.emoji };
    let maint_line = if product.is_maintenance {
        "🔧 Trạng thái: <b>Đang bảo trì</b>\n"
    } else {
        ""
    };
    let cat_line = product
        .category_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|c| format!("📂 Danh mục: <b>{}</b>\n", escape_html(c)))
        .unwrap_or_default();
    let desc_block = if product.description.trim().is_empty() {
        "Giao tài khoản / key tự động ngay lập tức sau khi thanh toán.".to_string()
    } else {
        escape_html(product.description.trim())
    };
    let type_label = if product.product_type.eq_ignore_ascii_case("key") {
        "Key kích hoạt"
    } else {
        "Tài khoản giao tự động"
    };

    format!(
        "{icon} <b>{}</b>\n\
         {cat_line}{maint_line}🏷️ Loại: <b>{type_label}</b>\n\
         💸 Giá bán: <b>{} đ</b>\n\
         📦 Tồn kho: <b>{} sản phẩm</b>\n\
         📊 Đã bán: <b>{}</b>\n\n\
         <blockquote>✏️ <b>Mô tả:</b>\n\
         {desc_block}</blockquote>\n\n\
         ⚡ Giao hàng tự động ngay lập tức 24/7.\n\
         🛟 Hỗ trợ: {}",
        escape_html(&product.name),
        format_vnd(product.sell_price_vnd),
        product.stock,
        product.sold_count,
        escape_html(support_handle),
    )
}

pub fn partner_product_maintenance_message(product: &PartnerProduct) -> String {
    let note = if product.maintenance_note.trim().is_empty() {
        "Vui lòng thử lại sau."
    } else {
        product.maintenance_note.trim()
    };
    format!(
        "🔧 <b>{} đang bảo trì</b>\n\
         ══════════════════════════\n\n\
         {}",
        escape_html(&product.name),
        escape_html(note),
    )
}
