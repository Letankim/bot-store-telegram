-- =============================================================================
-- Add per-plan API processing mode (autopay | payadd | extract)
-- =============================================================================

ALTER TABLE gemini_plans
    ADD COLUMN IF NOT EXISTS api_mode VARCHAR(32) NOT NULL DEFAULT 'autopay'
    CHECK (api_mode IN ('autopay', 'payadd', 'extract'));
