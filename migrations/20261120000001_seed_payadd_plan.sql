-- =============================================================================
-- Seed "Thêm PaySafeCard tự động" (mode = payadd) product plan
-- =============================================================================

INSERT INTO gemini_plans (
    code,
    name,
    duration_months,
    price_vnd,
    available_stock,
    sold_count,
    description_note,
    api_mode,
    is_active,
    display_order
)
VALUES (
    'PAYSAFE_ADD_AUTO',
    'Thêm PaySafeCard tự động (Add thẻ thanh toán)',
    1,
    20000,
    500,
    0,
    '• Tự động thêm phương thức thanh toán PaySafeCard vào tài khoản Google của bạn.
• Khách hàng cần cung cấp Mail|Pass|2FA (bắt buộc).
• Vui lòng đảm bảo đã bật xác minh 2 bước (2FA 32 ký tự) và đóng sạch toàn bộ hồ sơ thanh toán cũ trước khi gửi đơn.
• Chỉ thử lại tối đa 3 lần cho 1 tài khoản Gmail.
• Thông tin bạn cung cấp được mã hóa AES-256-GCM và bảo mật hoàn toàn.',
    'payadd',
    TRUE,
    3
)
ON CONFLICT (code) DO NOTHING;
