-- =============================================================================
-- Upstream Partner Pixel API (api.smm.io.vn) Settings & Order Tracking Columns
-- =============================================================================

ALTER TABLE app_settings
    ADD COLUMN IF NOT EXISTS smm_api_base_url VARCHAR(255) NOT NULL DEFAULT 'https://api.smm.io.vn',
    ADD COLUMN IF NOT EXISTS smm_api_key TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS smm_auto_push BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS smm_default_mode VARCHAR(32) NOT NULL DEFAULT 'autopay',
    ADD COLUMN IF NOT EXISTS smm_auto_refund_on_fail BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE activation_orders
    ADD COLUMN IF NOT EXISTS external_job_id BIGINT,
    ADD COLUMN IF NOT EXISTS external_status VARCHAR(32),
    ADD COLUMN IF NOT EXISTS external_step VARCHAR(64),
    ADD COLUMN IF NOT EXISTS external_result TEXT;
