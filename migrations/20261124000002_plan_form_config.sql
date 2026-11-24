-- =============================================================================
-- Configurable Customer Form Modes per Product (`gemini_plans`)
-- Supports: 'mail_pass_2fa' | 'email_only' | 'custom_note' | 'none'
-- Plus 'manual' api_mode for products fulfilled manually by Admin.
-- =============================================================================

ALTER TABLE gemini_plans
    ADD COLUMN IF NOT EXISTS form_mode VARCHAR(32) NOT NULL DEFAULT 'mail_pass_2fa',
    ADD COLUMN IF NOT EXISTS custom_field_label TEXT NOT NULL DEFAULT '';
