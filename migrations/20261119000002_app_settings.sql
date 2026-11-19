-- =============================================================================
-- App Settings Table (Singleton row id = 1, live-configurable from /admin)
-- =============================================================================

CREATE TABLE IF NOT EXISTS app_settings (
    id SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    support_handle VARCHAR(64) NOT NULL DEFAULT '@kim12ad',
    min_deposit_vnd BIGINT NOT NULL DEFAULT 10000 CHECK (min_deposit_vnd >= 1000),
    referral_reward_vnd BIGINT NOT NULL DEFAULT 5000 CHECK (referral_reward_vnd >= 0),
    referral_min_first_deposit_vnd BIGINT NOT NULL DEFAULT 10000 CHECK (referral_min_first_deposit_vnd >= 0),
    quick_deposit_amounts VARCHAR(128) NOT NULL DEFAULT '40000,80000,150000,300000',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO app_settings (
    id,
    support_handle,
    min_deposit_vnd,
    referral_reward_vnd,
    referral_min_first_deposit_vnd,
    quick_deposit_amounts
)
VALUES (1, '@kim12ad', 10000, 5000, 10000, '40000,80000,150000,300000')
ON CONFLICT (id) DO NOTHING;
