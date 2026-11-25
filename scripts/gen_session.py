"""
One-time helper to generate `TG_STRING_SESSION` for running `telegram_auto_sync.py`
24/7 on Render.com (no `.session` file needed on Render's ephemeral disk).

Usage:
  pip install -r scripts/requirements.txt
  python scripts/gen_session.py
"""

import asyncio
from telethon import TelegramClient
from telethon.sessions import StringSession


async def main() -> None:
    print("=== TẠO TG_STRING_SESSION ĐỂ CHẠY 24/7 TRÊN RENDER.COM ===")
    api_id = int(input("Nhập TG_API_ID (dãy số): ").strip())
    api_hash = input("Nhập TG_API_HASH: ").strip()

    async with TelegramClient(StringSession(), api_id, api_hash) as client:
        session_str = client.session.save()
        me = await client.get_me()
        print("\n✅ Đăng nhập thành công tài khoản:", getattr(me, "first_name", "User"), f"(ID: {me.id})")
        print("\n📋 Copy TOÀN BỘ chuỗi bên dưới dán vào biến TG_STRING_SESSION trên Render:")
        print("-" * 70)
        print(session_str)
        print("-" * 70)
        print("⚠️ Tuyệt đối giữ bí mật chuỗi này (không chia sẻ hay commit lên GitHub)!")


if __name__ == "__main__":
    asyncio.run(main())
