-- =============================================================================
-- Add referred_bonus_vnd (bonus for the invited friend on first deposit)
-- =============================================================================

ALTER TABLE app_settings
    ADD COLUMN IF NOT EXISTS referred_bonus_vnd BIGINT NOT NULL DEFAULT 2000 CHECK (referred_bonus_vnd >= 0);

ALTER TABLE referrals
    ADD COLUMN IF NOT EXISTS referred_bonus_vnd BIGINT NOT NULL DEFAULT 0 CHECK (referred_bonus_vnd >= 0);
