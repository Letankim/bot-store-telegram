use crate::domain::{gemini_plan::GeminiPlan, wallet::format_vnd};

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
    min_first_deposit_vnd: i64,
    support_handle: &str,
) -> String {
    format!(
        "👋 Xin chào <b>{}</b>!\n\n\
         🤖 Chào mừng bạn đến với hệ thống <b>Kích Hoạt Gemini Pro Chính Chủ (Your Mail)</b> tự động.\n\n\
         💰 Số dư ví hiện tại: <b>{} đ</b>\n\
         🎁 <b>Chương trình mời bạn bè:</b>\n\
         <code>{}</code>\n\
         <i>• Nhận ngay <b>{} đ</b> vào ví khi bạn bè đăng ký qua link và bắt buộc nạp tiền lần đầu tiên (tối thiểu {} đ).</i>\n\n\
         🛟 Hỗ trợ: {}\n\
         👇 Chọn chức năng bên dưới hoặc gõ /pixel để xem gói kích hoạt Gemini Pro!",
        escape_html(full_name),
        format_vnd(balance_vnd),
        escape_html(ref_link),
        format_vnd(reward_vnd),
        format_vnd(min_first_deposit_vnd),
        escape_html(support_handle),
    )
}

pub fn gemini_plan_card(plan: &GeminiPlan, guide_url: &str, support_handle: &str) -> String {
    format!(
        "🔹 <b>{}</b>\n\
         💸 Giá: <b>{} đ</b>\n\
         📦 Tồn kho: <b>{} tài khoản</b>\n\
         📊 Đã bán: <b>{} tài khoản</b>\n\n\
         <blockquote>✏️ <b>Mô tả:</b>\n\
         {}\n\
         - Hướng dẫn đóng HSTT &amp; gắn 2FA: <a href=\"{}\">Xem trang hướng dẫn chi tiết tại đây</a></blockquote>\n\n\
         ‼️ Vui lòng đọc kỹ tiêu đề sản phẩm &amp; mô tả trước khi mua hàng.\n\
         Chỉ hỗ trợ refund với những đơn hàng lỗi.\n\
         Hỗ trợ: {}",
        escape_html(&plan.name),
        format_vnd(plan.price_vnd),
        plan.available_stock,
        plan.sold_count,
        escape_html(&plan.description_note),
        escape_html(guide_url),
        escape_html(support_handle)
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
