"""
24/7 Automatic Telegram Partner Bot Listener -> BotStoreTelegram Auto-Sync

Listens automatically to incoming stock & price notifications from a partner Telegram bot
(using a secondary Telegram account via Telethon MTProto) and immediately pushes them to
`POST /api/v1/admin/partner-shop/ingest-message` on your BotStoreTelegram server.

Features:
  - Live 24/7 auto-sync on new partner bot messages (respects `telethon_auto_sync` toggle in /admin).
  - Built-in HTTP control server on `$PORT` (for Render Free Web Service):
      * GET  / or /health -> healthcheck JSON
      * POST /sync        -> on-demand history scan triggered by Admin clicking "Đồng bộ ngay" in /admin
                             (authenticated with `X-Admin-Api-Key`).

Usage:
  1. pip install -r scripts/requirements.txt
  2. Set env vars (or create scripts/.env):
       TG_API_ID=12345678
       TG_API_HASH=your_api_hash_from_my_telegram_org
       TG_STRING_SESSION=... (recommended for Render)
       PARTNER_BOT_USERNAMES=@bot_doi_tac
       SHOP_BASE_URL=https://gemini-pro-shop.onrender.com
       ADMIN_API_KEY=your_admin_api_key
  3. Run:
       python scripts/telegram_auto_sync.py
"""

import argparse
import asyncio
import hmac
import json
import logging
import os
import sys
import time
from pathlib import Path
from typing import Any

import requests
from dotenv import load_dotenv
from telethon import TelegramClient, events
from telethon.sessions import StringSession

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
log = logging.getLogger("auto_sync")


class SyncConfigCache:
    """Caches live auto-sync toggle & notify flag from /api/v1/admin/partner-shop/sync-config."""

    def __init__(self, base_url: str, admin_key: str, ttl_secs: float = 15.0) -> None:
        self.base_url = base_url.rstrip("/")
        self.admin_key = admin_key
        self.ttl_secs = ttl_secs
        self._last_fetch = 0.0
        self._telethon_auto_sync = True
        self._notify_changes = True

    def get(self) -> tuple[bool, bool]:
        now = time.monotonic()
        if now - self._last_fetch >= self.ttl_secs:
            self._refresh()
        return self._telethon_auto_sync, self._notify_changes

    def _refresh(self) -> None:
        url = f"{self.base_url}/api/v1/admin/partner-shop/sync-config"
        try:
            resp = requests.get(
                url,
                headers={"X-Admin-Api-Key": self.admin_key},
                timeout=8,
            )
            if resp.status_code == 200:
                payload = resp.json()
                data = payload.get("data") or {}
                if "telethon_auto_sync" in data:
                    self._telethon_auto_sync = bool(data["telethon_auto_sync"])
                if "partner_shop_notify_changes" in data:
                    self._notify_changes = bool(data["partner_shop_notify_changes"])
                self._last_fetch = time.monotonic()
        except Exception as exc:
            log.debug("Không thể tải sync-config (giữ cấu hình gần nhất): %s", exc)
            self._last_fetch = time.monotonic()


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


def push_to_shop(base_url: str, admin_key: str, text: str, notify_customers: bool = True) -> bool:
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
            return True
        log.warning("SYNC REJECTED (%s): %s", resp.status_code, data.get("message"))
        return False
    except Exception as exc:
        log.error("SYNC REQUEST ERROR: %s", exc)
        return False


async def scan_targets_history(
    client: TelegramClient,
    targets: list[str],
    shop_base_url: str,
    admin_api_key: str,
    limit: int,
    notify_customers: bool,
) -> dict[str, Any]:
    scanned = 0
    matched = 0
    synced = 0
    failed = 0

    for target in targets:
        try:
            # Iterate oldest -> newest so latest price/stock wins
            msgs = [m async for m in client.iter_messages(target, limit=limit)]
            scanned += len(msgs)
            for m in reversed(msgs):
                if m.raw_text and is_partner_product_alert(m.raw_text):
                    matched += 1
                    ok = await asyncio.to_thread(
                        push_to_shop, shop_base_url, admin_api_key, m.raw_text, notify_customers
                    )
                    if ok:
                        synced += 1
                    else:
                        failed += 1
        except Exception as exc:
            log.error("Lỗi quét lịch sử %s: %s", target, exc)
            failed += 1

    return {
        "ok": True,
        "scanned": scanned,
        "matched": matched,
        "synced": synced,
        "failed": failed,
        "message": (
            f"Bot đối tác: quét {scanned} tin, khớp {matched} sản phẩm, "
            f"đồng bộ thành công {synced} (lỗi {failed})"
        ),
    }


def _build_http_response(status: int, reason: str, payload: dict[str, Any]) -> bytes:
    body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    return (
        f"HTTP/1.1 {status} {reason}\r\n"
        "Content-Type: application/json; charset=utf-8\r\n"
        f"Content-Length: {len(body)}\r\n"
        "Connection: close\r\n\r\n"
    ).encode("ascii") + body


async def start_control_server(
    host: str,
    port: int,
    admin_api_key: str,
    targets: list[str],
    shop_base_url: str,
    client_holder: dict[str, Any],
    sync_lock: asyncio.Lock,
) -> asyncio.AbstractServer:
    """HTTP health & on-demand `/sync` endpoint for single-container sidecar or Render Web Service."""

    async def handle_client(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            raw_head = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5.0)
        except Exception:
            try:
                writer.close()
            except Exception:
                pass
            return

        try:
            head_text = raw_head.decode("utf-8", errors="replace")
            lines = head_text.split("\r\n")
            req_line = lines[0].split(" ") if lines else []
            method = req_line[0].upper() if len(req_line) >= 1 else "GET"
            path = req_line[1].split("?")[0] if len(req_line) >= 2 else "/"

            headers: dict[str, str] = {}
            for line in lines[1:]:
                if ":" in line:
                    k, v = line.split(":", 1)
                    headers[k.strip().lower()] = v.strip()

            content_len = int(headers.get("content-length", "0") or "0")
            body_bytes = b""
            if content_len > 0 and content_len <= 65536:
                body_bytes = await asyncio.wait_for(reader.readexactly(content_len), timeout=5.0)

            if method == "POST" and path == "/sync":
                provided_key = headers.get("x-admin-api-key", "")
                if not hmac.compare_digest(provided_key, admin_api_key):
                    writer.write(
                        _build_http_response(
                            401, "Unauthorized", {"ok": False, "message": "Sai X-Admin-Api-Key"}
                        )
                    )
                    await writer.drain()
                    return

                tg_client = client_holder.get("client")
                if tg_client is None or not tg_client.is_connected():
                    writer.write(
                        _build_http_response(
                            503,
                            "Service Unavailable",
                            {"ok": False, "message": "Telegram Client chưa kết nối xong, thử lại sau 5 giây."},
                        )
                    )
                    await writer.drain()
                    return

                req_json: dict[str, Any] = {}
                if body_bytes:
                    try:
                        req_json = json.loads(body_bytes.decode("utf-8"))
                    except Exception:
                        req_json = {}

                limit = max(5, min(100, int(req_json.get("limit", 30) or 30)))
                notify = bool(req_json.get("notify_customers", False))

                if sync_lock.locked():
                    writer.write(
                        _build_http_response(
                            429,
                            "Too Many Requests",
                            {"ok": False, "message": "Đang có tiến trình đồng bộ khác chạy, vui lòng đợi vài giây."},
                        )
                    )
                    await writer.drain()
                    return

                async with sync_lock:
                    log.info("Nhận lệnh Đồng bộ thủ công từ /admin (limit=%d, notify=%s)", limit, notify)
                    result = await scan_targets_history(
                        tg_client, targets, shop_base_url, admin_api_key, limit, notify
                    )

                writer.write(_build_http_response(200, "OK", result))
                await writer.drain()
                return

            # Default GET / or /health
            writer.write(
                _build_http_response(
                    200,
                    "OK",
                    {
                        "ok": True,
                        "service": "telegram-auto-sync",
                        "connected": bool(client_holder.get("client") and client_holder["client"].is_connected()),
                        "targets": targets,
                    },
                )
            )
            await writer.drain()
        except Exception as exc:
            log.debug("HTTP handler error: %s", exc)
        finally:
            try:
                writer.close()
            except Exception:
                pass

    server = await asyncio.start_server(handle_client, host, port)
    log.info("Control & Health HTTP server listening on %s:%d (GET /health, POST /sync)", host, port)
    return server


def wait_for_shop_ready(shop_base_url: str, max_wait_secs: int = 30) -> None:
    """Wait briefly until the co-located Rust Axum server finishes booting & running migrations."""
    health_url = f"{shop_base_url.rstrip('/')}/api/v1/health"
    deadline = time.monotonic() + max_wait_secs
    while time.monotonic() < deadline:
        try:
            r = requests.get(health_url, timeout=3)
            if r.status_code == 200:
                return
        except Exception:
            pass
        time.sleep(1.5)


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
    auto_sync_port_raw = os.getenv("AUTO_SYNC_PORT", "").strip()
    port_raw = os.getenv("PORT", "").strip()

    if not api_id_raw or not api_hash or not partners_raw or not admin_api_key:
        log.error(
            "Thiếu biến môi trường bắt buộc: TG_API_ID, TG_API_HASH, PARTNER_BOT_USERNAMES, ADMIN_API_KEY"
        )
        sys.exit(1)

    api_id = int(api_id_raw)
    targets = [p.strip() for p in partners_raw.split(",") if p.strip()]
    client_holder: dict[str, Any] = {"client": None}
    sync_lock = asyncio.Lock()
    config_cache = SyncConfigCache(shop_base_url, admin_api_key, ttl_secs=15.0)

    if auto_sync_port_raw.isdigit():
        await start_control_server(
            "127.0.0.1",
            int(auto_sync_port_raw),
            admin_api_key,
            targets,
            shop_base_url,
            client_holder,
            sync_lock,
        )
    elif port_raw.isdigit():
        await start_control_server(
            "0.0.0.0",
            int(port_raw),
            admin_api_key,
            targets,
            shop_base_url,
            client_holder,
            sync_lock,
        )

    if string_session:
        client = TelegramClient(StringSession(string_session), api_id, api_hash)
        await client.connect()
        if not await client.is_user_authorized():
            log.error("TG_STRING_SESSION không hợp lệ hoặc đã bị thu hồi! Hãy chạy lại scripts/gen_session.py.")
            sys.exit(1)
    else:
        client = TelegramClient(session_name, api_id, api_hash)
        await client.start()

    client_holder["client"] = client
    me = await client.get_me()
    log.info("Đã kết nối Telegram Client: %s (ID: %s)", getattr(me, "first_name", "User"), me.id)
    log.info("Đang theo dõi tự động 24/7 các bot/kênh: %s", ", ".join(targets))

    if scan_count > 0:
        await asyncio.to_thread(wait_for_shop_ready, shop_base_url, 30)
        log.info("Đang quét %d tin nhắn gần nhất để nhập sản phẩm hiện có...", scan_count)
        async with sync_lock:
            res = await scan_targets_history(
                client, targets, shop_base_url, admin_api_key, scan_count, False
            )
            log.info("Kết quả quét lúc khởi động: %s", res["message"])

    @client.on(events.NewMessage(chats=targets))
    async def on_partner_message(event: events.NewMessage.Event) -> None:
        text = event.raw_text or ""
        if not is_partner_product_alert(text):
            return
        auto_enabled, notify_customers = await asyncio.to_thread(config_cache.get)
        if not auto_enabled:
            log.info("Tự động sync Bot đối tác đang TẮT trong /admin — bỏ qua tin nhắn này.")
            return
        log.info("Phát hiện thông báo sản phẩm mới từ %s — đang tự động đồng bộ...", event.chat_id)
        await asyncio.to_thread(push_to_shop, shop_base_url, admin_api_key, text, notify_customers)

    log.info("Listener 24/7 đã sẵn sàng! Mọi cập nhật giá & kho sẽ tự động đẩy lên shop.")
    await client.run_until_disconnected()


if __name__ == "__main__":
    asyncio.run(main())
