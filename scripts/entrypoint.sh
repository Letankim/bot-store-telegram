#!/bin/sh
set -eu

# Auto-start built-in Telethon Partner Bot Auto-Sync inside the same Render container
# when Telegram userbot env vars are configured.
if [ -n "${TG_API_ID:-}" ] && [ -n "${TG_API_HASH:-}" ] && [ -n "${TG_STRING_SESSION:-}" ] && [ -n "${PARTNER_BOT_USERNAMES:-}" ]; then
  echo "[entrypoint] Starting built-in Telethon Auto-Sync sidecar on 127.0.0.1:${AUTO_SYNC_PORT:-18080}..."
  (
    export SHOP_BASE_URL="${SHOP_BASE_URL:-http://127.0.0.1:${PORT:-10000}}"
    export AUTO_SYNC_PORT="${AUTO_SYNC_PORT:-18080}"
    while true; do
      /app/.venv/bin/python /app/scripts/telegram_auto_sync.py || true
      echo "[entrypoint] Telethon Auto-Sync exited — restarting in 10s..."
      sleep 10
    done
  ) &
else
  echo "[entrypoint] Telethon Auto-Sync env vars not set — running Rust server only."
fi

exec /app/bot_store_telegram
