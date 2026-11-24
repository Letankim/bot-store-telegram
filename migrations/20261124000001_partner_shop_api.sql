-- =============================================================================
-- Partner Shop API Integration (telebot.aidesign.io.vn/api/partner-api)
-- Auto-sync products, configurable profit markup (default +30,000 VND),
-- instant account/key delivery with AES-256-GCM storage & ACID wallet ledger.
-- =============================================================================

-- 1. Extend app_settings with Partner Shop API configuration
ALTER TABLE app_settings
    ADD COLUMN IF NOT EXISTS partner_shop_base_url TEXT NOT NULL DEFAULT 'https://telebot.aidesign.io.vn/api/partner-api',
    ADD COLUMN IF NOT EXISTS partner_shop_api_key TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS partner_shop_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS partner_shop_markup_vnd BIGINT NOT NULL DEFAULT 30000 CHECK (partner_shop_markup_vnd >= 0),
    ADD COLUMN IF NOT EXISTS partner_shop_sync_interval_secs INT NOT NULL DEFAULT 180 CHECK (partner_shop_sync_interval_secs >= 30),
    ADD COLUMN IF NOT EXISTS partner_shop_notify_changes BOOLEAN NOT NULL DEFAULT TRUE;

-- 2. Extend wallet_transactions tx_type CHECK constraint to allow PARTNER_SHOP_PURCHASE
ALTER TABLE wallet_transactions
    DROP CONSTRAINT IF EXISTS wallet_transactions_tx_type_check;

ALTER TABLE wallet_transactions
    ADD CONSTRAINT wallet_transactions_tx_type_check CHECK (
        tx_type IN (
            'PAYOS_TOPUP',
            'GEMINI_ACTIVATION',
            'PARTNER_SHOP_PURCHASE',
            'ORDER_REFUND',
            'REFERRAL_REWARD',
            'ADMIN_ADJUST'
        )
    );

-- 3. Synced Partner Shop products table
CREATE TABLE IF NOT EXISTS partner_products (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    external_id BIGINT NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    product_type VARCHAR(32) NOT NULL DEFAULT 'account',
    emoji VARCHAR(32) NOT NULL DEFAULT '',
    image_url TEXT,
    category_name VARCHAR(128),
    category_emoji VARCHAR(32),
    category_sort INT NOT NULL DEFAULT 0,
    partner_price_vnd BIGINT NOT NULL DEFAULT 0 CHECK (partner_price_vnd >= 0),
    custom_markup_vnd BIGINT CHECK (custom_markup_vnd IS NULL OR custom_markup_vnd >= 0),
    sell_price_vnd BIGINT NOT NULL DEFAULT 0 CHECK (sell_price_vnd >= 0),
    stock INT NOT NULL DEFAULT 0 CHECK (stock >= 0),
    sold_count INT NOT NULL DEFAULT 0 CHECK (sold_count >= 0),
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    is_maintenance BOOLEAN NOT NULL DEFAULT FALSE,
    maintenance_note TEXT NOT NULL DEFAULT 'Vui lòng thử lại sau.',
    last_synced_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_partner_products_active_cat
    ON partner_products(is_active, category_sort ASC, external_id ASC);

-- 4. Partner Shop instant purchase orders (delivered items encrypted with AES-256-GCM)
CREATE TABLE IF NOT EXISTS partner_orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_code VARCHAR(32) NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    partner_product_id UUID NOT NULL REFERENCES partner_products(id),
    external_product_id BIGINT NOT NULL,
    product_name VARCHAR(255) NOT NULL,
    quantity INT NOT NULL CHECK (quantity > 0),
    unit_partner_price_vnd BIGINT NOT NULL CHECK (unit_partner_price_vnd >= 0),
    unit_sell_price_vnd BIGINT NOT NULL CHECK (unit_sell_price_vnd >= 0),
    total_partner_cost_vnd BIGINT NOT NULL CHECK (total_partner_cost_vnd >= 0),
    total_sell_price_vnd BIGINT NOT NULL CHECK (total_sell_price_vnd > 0),
    profit_vnd BIGINT NOT NULL DEFAULT 0,
    items_encrypted BYTEA,
    nonce BYTEA,
    status VARCHAR(32) NOT NULL DEFAULT 'PROCESSING' CHECK (
        status IN ('PROCESSING', 'COMPLETED', 'REFUNDED')
    ),
    error_note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_partner_orders_user ON partner_orders(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_partner_orders_status ON partner_orders(status, created_at DESC);
