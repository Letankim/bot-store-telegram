"""
24/7 Automatic Telegram Partner Bot Listener -> BotStoreTelegram Auto-Sync

Listens automatically to incoming stock & price notifications from a partner Telegram bot
(using a secondary Telegram account via Telethon MTProto) and immediately pushes them to
`POST /api/v1/admin/partner-shop/ingest-message` on your BotStoreTelegram server.

Your server automatically:
  1. Parses product name, partner cost price, added quantity, and current stock.
  2. Adds your configured profit markup (default +30,000 VND in /admin).
  3. Creates or updates the product on both your Telegram Bot & Website.
  4. Broadcasts restock / price-change notifications to all active customers.

Usage:
  1. pip install -r scripts/requirements.txt
  2. Set env vars (or create scripts/.env):
       TG_API_ID=12345678
       TG_API_HASH=your_api_hash_from_my_telegram_org
       PARTNER_BOT_USERNAMES=@bot_doi_tac
       SHOP_BASE_URL=https://gemini-pro-shop.onrender.com
       ADMIN_API_KEY=your_admin_api_key
  3. Run:
       python scripts/telegram_auto_sync.py
     Optional: scan last 20 messages on startup to import existing products without broadcasting:
       python scripts/telegram_auto_sync.py --scan-history 20
"""

import argparse
import asyncio
import logging
import os
import sys
from pathlib import Path

import requests
from dotenv import load_dotenv
from telethon import TelegramClient, events
from telethon.sessions import StringSession

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
log = logging.getLogger("auto_sync")


async def start_health_server(port: int) -> asyncio.AbstractServer:
    """Tiny HTTP health endpoint so this listener can run 24/7 on a FREE Render Web Service."""
    async def handle_client(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            await asyncio.wait_for(reader.read(1024), timeout=5.0)
            body = b'{"ok":true,"service":"telegram-auto-sync"}'
            resp = (
                b"HTTP/1.1 200 OK\r\n"
                b"Content-Type: application/json\r\n"
                + f"Content-Length: {len(body)}\r\n".encode("ascii")
                + b"Connection: close\r\n\r\n"
                + body
            )
            writer.write(resp)
            await writer.drain()
        except Exception:
            pass
        finally:
            writer.close()

    server = await asyncio.start_server(handle_client, "0.0.0.0", port)
    log.info("Health HTTP server listening on 0.0.0.0:%d (Render Free Web Service ready)", port)
    return server


def is_partner_product_alert(text: str) -> bool:
    if not text:
        return False
    low = text.lower()
    has_price = any(k in low for k in ("giá:", "gia:", "giá mới:", "giá bán:"))
    has_product_or_stock = any(
        k in low
        for k in (
            "sản phẩm:",
            "san pham:",
            "tên gói:",
            "gói:",
            "còn hàng:",
            "con hang:",
            "kho hiện tại:",
            "tồn kho:",
        )
    )
    return has_price and has_product_or_stock


def push_to_shop(base_url: str, admin_key: str, text: str, notify_customers: bool = True) -> None:
    url = f"{base_url.rstrip('/')}/api/v1/admin/partner-shop/ingest-message"
    try:
        resp = requests.post(
            url,
            headers={
                "X-Admin-Api-Key": admin_key,
                "Content-Type": "application/json",
            },
            json={
                "text": text,
                "notify_customers": notify_customers,
            },
            timeout=20,
        )
        data = resp.json()
        if data.get("success"):
            log.info("SYNC OK: %s", data.get("message"))
        else:
            log.warning("SYNC REJECTED (%s): %s", resp.status_code, data.get("message"))
    except Exception as exc:
        log.error("SYNC REQUEST ERROR: %s", exc)


async def main() -> None:
    parser = argparse.ArgumentParser(description="24/7 Partner Bot -> Shop Auto-Sync Listener")
    parser.add_argument(
        "--scan-history",
        type=int,
        default=0,
        help="Optional: scan the latest N messages from partner bot(s) on startup (silent import).",
    )
    args = parser.parse_args()

    # Load scripts/.env if present, fallback to root .env
    script_env = Path(__file__).parent / ".env"
    if script_env.exists():
        load_dotenv(script_env)
    else:
        load_dotenv()

    api_id_raw = os.getenv("TG_API_ID", "").strip()
    api_hash = os.getenv("TG_API_HASH", "").strip()
    string_session = os.getenv("TG_STRING_SESSION", "").strip()
    partners_raw = os.getenv("PARTNER_BOT_USERNAMES", "").strip()
    shop_base_url = os.getenv("SHOP_BASE_URL", "https://gemini-pro-shop.onrender.com").strip()
    admin_api_key = os.getenv("ADMIN_API_KEY", "").strip()
    session_name = os.getenv("TG_SESSION_PATH", str(Path(__file__).parent / "partner_listener"))
    scan_history_env = int(os.getenv("SCAN_HISTORY_ON_BOOT", "0") or "0")
    scan_count = args.scan_history or scan_history_env
    port_raw = os.getenv("PORT", "").strip()

    if not api_id_raw or not api_hash or not partners_raw or not admin_api_key:
        log.error(
            "Thiếu biến môi trường bắt buộc: TG_API_ID, TG_API_HASH, PARTNER_BOT_USERNAMES, ADMIN_API_KEY"
        )
        sys.exit(1)

    if port_raw.isdigit():
        await start_health_server(int(port_raw))

    api_id = int(api_id_raw)
    targets = [p.strip() for p in partners_raw.split(",") if p.strip()]

    if string_session:
        client = TelegramClient(StringSession(string_session), api_id, api_hash)
        await client.connect()
        if not await client.is_user_authorized():
            log.error("TG_STRING_SESSION không hợp lệ hoặc đã bị thu hồi! Hãy chạy lại scripts/gen_session.py.")
            sys.exit(1)
    else:
        client = TelegramClient(session_name, api_id, api_hash)
        await client.start()

    me = await client.get_me()
    log.info("Đã kết nối Telegram Client: %s (ID: %s)", getattr(me, "first_name", "User"), me.id)
    log.info("Đang theo dõi tự động 24/7 các bot/kênh: %s", ", ".join(targets))

    if scan_count > 0:
        log.info("Đang quét %d tin nhắn gần nhất để nhập sản phẩm hiện có...", scan_count)
        for target in targets:
            try:
                # Iterate oldest -> newest so latest price/stock wins
                msgs = [m async for m in client.iter_messages(target, limit=scan_count)]
                for m in reversed(msgs):
                    if m.raw_text and is_partner_product_alert(m.raw_text):
                        await asyncio.to_thread(
                            push_to_shop, shop_base_url, admin_api_key, m.raw_text, False
                        )
            except Exception as exc:
                log.error("Lỗi quét lịch sử %s: %s", target, exc)

    @client.on(events.NewMessage(chats=targets))
    async def on_partner_message(event: events.NewMessage.Event) -> None:
        text = event.raw_text or ""
        if not is_partner_product_alert(text):
            return
        log.info("Phát hiện thông báo sản phẩm mới từ %s — đang tự động đồng bộ...", event.chat_id)
        await asyncio.to_thread(push_to_shop, shop_base_url, admin_api_key, text, True)

    log.info("Listener 24/7 đã sẵn sàng! Mọi cập nhật giá & kho sẽ tự động đẩy lên shop.")
    await client.run_until_disconnected()


if __name__ == "__main__":
    asyncio.run(main())
