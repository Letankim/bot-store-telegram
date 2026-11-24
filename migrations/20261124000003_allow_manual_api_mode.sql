-- =============================================================================
-- Allow 'manual' in gemini_plans.api_mode CHECK constraint for manual/email-only products
-- =============================================================================

ALTER TABLE gemini_plans
    DROP CONSTRAINT IF EXISTS gemini_plans_api_mode_check;

ALTER TABLE gemini_plans
    ADD CONSTRAINT gemini_plans_api_mode_check
    CHECK (api_mode IN ('autopay', 'payadd', 'extract', 'manual'));
