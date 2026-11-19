---
name: telegram-bot-flow
description: >-
  Use this skill when building or updating Teloxide Telegram bot commands, Gemini Pro product card templates, Telegram Mini App (WebApp) form buttons, and the step-by-step activation guide page.
---

# Telegram Bot & Mini App Activation Flow Skill (`telegram-bot-flow`)

Sử dụng skill này khi lập trình luồng tương tác Telegram Bot (`src/bot/`) và giao diện Telegram Mini App (`web/activate.html`, `web/guide.html`).

---

## 1. Danh sách Slash Commands Chuẩn (`BotCommands`)

- `/start [ref_code]` — `▶️ Xem menu chính`
- `/pixel` (hoặc `/gemini`) — `🤖 Kích hoạt Gemini Pro chính chủ`
- `/payadd` — `💳 Nạp tiền tự động qua PayOS (VietQR)`
- `/guide` — `📘 Hướng dẫn đóng HSTT, kiểm tra Quốc gia & lấy mã 2FA`

---

## 2. Mẫu Tin nhắn Chi tiết Gói Gemini Pro (HTML ParseMode)

Hiển thị đúng 100% bố cục và blockquote như ảnh mẫu khi khách chọn xem gói kích hoạt Gemini Pro:

```html
🔹 <b>{plan_name}</b>
💸 Giá: <b>{price_formatted} đ</b>
📦 Tồn kho: <b>{available_stock} tài khoản</b>
📊 Đã bán: <b>{sold_count} tài khoản</b>

<blockquote>✏️ <b>Mô tả:</b>
• Trả đơn trong 1-6h hoặc sớm hơn, vui lòng không hối thúc.
- Chỉ nhận Gmail Việt Nam, nếu khách hàng sử dụng mail ngoại shop sẽ gửi link khách hàng tự kích hoạt bằng thẻ visa 0 đồng.
- Kiểm tra quốc gia gmail tại Link: <a href="https://policies.google.com/terms">https://policies.google.com/terms</a>
Cách xem: Ngay dưới dòng tiêu đề Điều khoản dịch vụ (Terms of Service), nhìn dòng Phiên bản quốc gia (Country version) — đây là quốc gia chính thức mà Google đang gán cho tài khoản của bạn.
• Khách hàng cần cung cấp Mail|Pass|2FA (bắt buộc)
• Admin familly add được 5 slot
- GeminiPro + 5TB + 1000 crd flow
- Full model anti: neon, hydrogen, opus 5.5,...
• Gmail phải là tài khoản trắng, chưa tham gia gói Google One / GeminiPro / Pixel nào, không ở trong family, và cần được chuẩn bị theo hướng dẫn.
- Hướng dẫn đóng HSTT & gắn 2FA: <a href="{guide_url}">Xem trang hướng dẫn chi tiết tại đây</a></blockquote>

‼️ Vui lòng đọc kỹ tiêu đề sản phẩm & mô tả trước khi mua hàng. Chỉ hỗ trợ refund với những đơn hàng lỗi.
Hỗ trợ: {support_handle}
```

### Các Nút Bấm Inline Dưới Tin Nhắn Gói Gemini Pro:
1. Hàng 1: `[ 🚀 Kiểm tra số dư & Kích hoạt ngay ]` (`callback_data = "chk_act:{plan_id}"`)
2. Hàng 2: `[ 📘 Xem Hướng dẫn Đóng HSTT & Lấy 2FA ]` (Nút `WebApp` hoặc `Url` mở trực tiếp trang `/webapp/guide`)
3. Hàng 3: `[ ⏪ Chọn gói khác ]` (`callback_data = "back:plans"`)

---

## 3. Luồng Kiểm tra Số dư & Mở Form Mini App (`chk_act:{plan_id}`)

Khi người dùng bấm `[ 🚀 Kiểm tra số dư & Kích hoạt ngay ]`:
- **Trường hợp 1: Số dư ví `< Giá gói`**:
  - Bot trả lời:
    `⚠️ Số dư ví của bạn hiện có {balance} đ (Cần {price} đ). Vui lòng nạp thêm {missing} đ qua PayOS để tiếp tục!`
  - Đính kèm nút `[ 💳 Nạp ngay {missing} đ (VietQR PayOS) ]` (`callback_data = "payos_create:{missing}"`).
- **Trường hợp 2: Số dư ví `>= Giá gói`**:
  - Bot trả lời:
    `✅ Số dư khả dụng ({balance} đ). Bấm nút bên dưới để mở Form điền thông tin Gmail | Mật khẩu | Mã 2FA và xem hướng dẫn từng bước:`
  - Đính kèm nút **Telegram Mini App (`InlineKeyboardButton::web_app`)** trỏ tới:
    `{PUBLIC_BASE_URL}/webapp/activate?plan_id={plan_id}`

---

## 4. Thiết kế Form Mini App (`/webapp/activate`) & Trang Hướng Dẫn (`/webapp/guide`)

### A. Tính năng của Form Mini App (`web/activate.html`)
1. **Chế độ Nhập Từng Ô (Guided Mode) & Chế độ Dán Nhanh (Quick Paste Mode)**:
   - **Ô 1 — Gmail cần nâng cấp**:
     - Placeholder: `customer1@gmail.com`
     - Nút trợ giúp nhỏ ngay cạnh nhãn: *"Cách kiểm tra Gmail Quốc gia Việt Nam & Tài khoản trắng"* (bấm vào mở modal hướng dẫn kèm link `https://policies.google.com/terms`).
   - **Ô 2 — Mật khẩu Gmail**:
     - Placeholder: `abcxyz`
     - Nút trợ giúp: *"Lưu ý mật khẩu chính xác để hệ thống đăng nhập kích hoạt tự động"*.
   - **Ô 3 — Mã khóa 2FA (32 ký tự Base32)**:
     - Placeholder: `ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl`
     - Tự động chuẩn hóa chữ thường, nhóm 4 ký tự cách nhau 1 khoảng trắng, kiểm tra định dạng Base32 hợp lệ (32 ký tự chữ `a-z` và số `2-7`) và **hiển thị mã OTP 6 số trực tiếp trên màn hình** để khách tự kiểm chứng mã 2FA của mình đã đúng chuẩn Authenticator hay chưa!
   - **Ô Dán Nhanh (Tùy chọn)**:
     - Cho phép khách dán nguyên dòng `customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl` $\rightarrow$ tự động tách vào 3 ô trên.
   - **Khung Xem Trước Định Dạng Chuẩn (Live Output Preview)**:
     - Hiển thị rõ chuỗi sẽ gửi đi:
       `customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl`
   - **Checklist Xác nhận trước khi Bấm Gửi**:
     - `[x] Tôi đã kiểm tra Gmail thuộc quốc gia Việt Nam và là tài khoản trắng (không ở trong Family).`
     - `[x] Tôi đã đóng Hồ sơ thanh toán (HSTT) cũ và bật 2FA lấy mã 32 ký tự theo đúng hướng dẫn.`

### B. Trang Hướng dẫn Toàn diện (`/webapp/guide`)
Trang web chuẩn Mobile-First (tích hợp theme Telegram WebApp) hướng dẫn trực quan từng bước có nút bấm mở nhanh cài đặt Google:
1. **Phần 1: Kiểm tra Quốc gia Tài khoản Google (Bắt buộc Việt Nam)**
   - Nút mở nhanh: `https://policies.google.com/terms`
   - Hình minh họa & chỉ dẫn: Xem dòng **Phiên bản quốc gia (Country version)** ngay dưới tiêu đề.
2. **Phần 2: Kiểm tra Rời Nhóm Gia đình (Family) & Đóng Hồ sơ Thanh toán (HSTT)**
   - Nút mở nhanh Family: `https://myaccount.google.com/family/details`
   - Nút mở nhanh Google Pay Settings: `https://pay.google.com/gp/w/u/0/home/settings`
   - Hướng dẫn kéo xuống cuối trang $\rightarrow$ bấm **"Đóng hồ sơ thanh toán" (Close payments profile)** để đưa Gmail về trạng thái tài khoản trắng.
3. **Phần 3: Cách Bật Xác Minh 2 Bước & Lấy Chuỗi Khóa 2FA 32 Ký Tự (`ydwx xcpo...`)**
   - Nút mở nhanh 2FA Google: `https://myaccount.google.com/signinoptions/two-step-verification`
   - Hướng dẫn chi tiết:
     1. Chọn mục **Ứng dụng xác thực (Authenticator app)** $\rightarrow$ Bấm **Thiết lập ứng dụng xác thực**.
     2. Khi màn hình hiện mã QR, **KHÔNG quét mã** mà bấm vào dòng chữ xanh **"Không thể quét mã?" (Can't scan it?)** ở ngay bên dưới mã QR.
     3. Google sẽ hiện ra chuỗi 32 ký tự có dạng: `ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl`.
     4. Copy toàn bộ chuỗi 32 ký tự đó, bấm **Tiếp theo**, nhập mã 6 số để hoàn tất lưu trên Google, rồi dán chuỗi 32 ký tự đó vào Form Kích Hoạt của Bot!
