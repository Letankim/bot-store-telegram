-- Auto-sync worker URL & Telethon auto-sync toggle for Admin UI control
ALTER TABLE app_settings
    ADD COLUMN IF NOT EXISTS auto_sync_worker_url TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS telethon_auto_sync BOOLEAN NOT NULL DEFAULT TRUE;
