# Hướng dẫn chạy tự động lắng nghe Bot đối tác 24/7 (không cần Forward tay)

Script [`telegram_auto_sync.py`](./telegram_auto_sync.py) tự động lắng nghe mọi tin nhắn thông báo lên kho / đổi giá từ Bot đối tác bằng tài khoản Telegram (khuyên dùng **nick Telegram phụ**) và tự đẩy về API `POST /api/v1/admin/partner-shop/ingest-message` của shop bạn ngay trong vòng 1 giây.

## Bước 1: Lấy `TG_API_ID` và `TG_API_HASH` (chỉ làm 1 lần)
1. Truy cập https://my.telegram.org và đăng nhập bằng số điện thoại Telegram (khuyên dùng nick phụ đã tham gia bot đối tác).
2. Vào mục **API development tools** → tạo 1 app mới → copy **`App api_id`** và **`App api_hash`**.

## Bước 2: Cài thư viện Python & tạo file `scripts/.env`
```bash
pip install -r scripts/requirements.txt
```

Tạo file `scripts/.env`:
```env
TG_API_ID=12345678
TG_API_HASH=abcdef1234567890abcdef1234567890
PARTNER_BOT_USERNAMES=@username_bot_doi_tac
SHOP_BASE_URL=https://gemini-pro-shop.onrender.com
ADMIN_API_KEY=your_admin_api_key
```

## Bước 3: Chạy lắng nghe tự động 24/7
- Chạy lần đầu (nhập số điện thoại & mã OTP Telegram 1 lần duy nhất để lưu file session; thêm `--scan-history 30` nếu muốn quét luôn 30 tin nhắn gần nhất để nhập sản phẩm sẵn có):
```bash
python scripts/telegram_auto_sync.py --scan-history 30
```
- Từ các lần sau trở đi chỉ cần chạy:
```bash
python scripts/telegram_auto_sync.py
```

Mỗi khi Bot đối tác bắn tin **`📢 HÀNG MỚI VỪA LÊN KHO`** hoặc đổi giá:
- Shop của bạn tự động lấy **Tên sản phẩm**, **Giá gốc + 30.000đ** (hoặc mức lời bạn chỉnh trong `/admin` → `Shop đối tác (API)`), **Tồn kho hiện tại**.
- Tự động cập nhật lên cả **Telegram Bot** & **Website** và tự động bắn thông báo tới toàn bộ khách hàng!
