-- =============================================================================
-- BotStoreTelegram — Initial PostgreSQL Schema & Seed Data
-- =============================================================================

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- 1. Users table
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    telegram_id BIGINT NOT NULL UNIQUE,
    username VARCHAR(128),
    full_name VARCHAR(255) NOT NULL,
    ref_code VARCHAR(32) NOT NULL UNIQUE,
    referred_by UUID REFERENCES users(id) ON DELETE SET NULL,
    role VARCHAR(16) NOT NULL DEFAULT 'USER' CHECK (role IN ('USER', 'ADMIN')),
    is_banned BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_users_telegram_id ON users(telegram_id);
CREATE INDEX IF NOT EXISTS idx_users_ref_code ON users(ref_code);

-- 2. Wallets table (Integer VND for 100% financial precision)
CREATE TABLE IF NOT EXISTS wallets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    balance_vnd BIGINT NOT NULL DEFAULT 0 CHECK (balance_vnd >= 0),
    total_deposited_vnd BIGINT NOT NULL DEFAULT 0 CHECK (total_deposited_vnd >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 3. Immutable Wallet Ledger (Append-Only)
CREATE TABLE IF NOT EXISTS wallet_transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_id UUID NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    tx_type VARCHAR(32) NOT NULL CHECK (
        tx_type IN ('PAYOS_TOPUP', 'GEMINI_ACTIVATION', 'ORDER_REFUND', 'REFERRAL_REWARD', 'ADMIN_ADJUST')
    ),
    amount_vnd BIGINT NOT NULL,
    balance_before BIGINT NOT NULL,
    balance_after BIGINT NOT NULL,
    idempotency_key VARCHAR(128) NOT NULL UNIQUE,
    description TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_wallet_tx_wallet_created ON wallet_transactions(wallet_id, created_at DESC);

-- 4. Referrals table (Reward on first deposit)
CREATE TABLE IF NOT EXISTS referrals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    referrer_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    referred_user_id UUID NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    reward_vnd BIGINT NOT NULL DEFAULT 5000 CHECK (reward_vnd >= 0),
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING_FIRST_DEPOSIT' CHECK (
        status IN ('PENDING_FIRST_DEPOSIT', 'REWARDED')
    ),
    rewarded_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 5. PayOS Deposits table
CREATE TABLE IF NOT EXISTS payos_deposits (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_code BIGINT NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    amount_vnd BIGINT NOT NULL CHECK (amount_vnd > 0),
    description VARCHAR(64) NOT NULL,
    checkout_url TEXT,
    qr_code TEXT,
    payment_link_id VARCHAR(128),
    reference VARCHAR(128),
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING' CHECK (
        status IN ('PENDING', 'PAID', 'CANCELLED', 'EXPIRED')
    ),
    paid_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_payos_deposits_user_status ON payos_deposits(user_id, status);

-- 6. Gemini Pro Activation Plans
CREATE TABLE IF NOT EXISTS gemini_plans (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code VARCHAR(64) NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL,
    duration_months INT NOT NULL CHECK (duration_months > 0),
    price_vnd BIGINT NOT NULL CHECK (price_vnd > 0),
    available_stock INT NOT NULL DEFAULT 0 CHECK (available_stock >= 0),
    sold_count INT NOT NULL DEFAULT 0 CHECK (sold_count >= 0),
    description_note TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    display_order INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 7. Activation Orders (Encrypted Mail | Pass | 2FA with AES-256-GCM)
CREATE TABLE IF NOT EXISTS activation_orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_code VARCHAR(32) NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    plan_id UUID NOT NULL REFERENCES gemini_plans(id),
    price_paid_vnd BIGINT NOT NULL CHECK (price_paid_vnd > 0),
    gmail_masked VARCHAR(255) NOT NULL,
    credentials_encrypted BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING' CHECK (
        status IN ('PENDING', 'PROCESSING', 'COMPLETED', 'REFUNDED')
    ),
    admin_note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_activation_orders_user ON activation_orders(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_activation_orders_status ON activation_orders(status, created_at ASC);

-- =============================================================================
-- Seed Default Gemini Pro Plans
-- =============================================================================
INSERT INTO gemini_plans (code, name, duration_months, price_vnd, available_stock, sold_count, description_note, display_order)
VALUES
(
    'GEMINI_PRO_3M_XIAOMI',
    'Gemini Pro Your Mail 3 tháng Xiaomi (NW) KBH',
    3,
    40000,
    395,
    99,
    '• Trả đơn trong 1-6h hoặc sớm hơn, vui lòng không hối thúc.
- Chỉ nhận Gmail Việt Nam, nếu khách hàng sử dụng mail ngoại shop sẽ gửi link khách hàng tự kích hoạt bằng thẻ visa 0 đồng.
- Kiểm tra quốc gia gmail tại Link: https://policies.google.com/terms
Cách xem: Ngay dưới dòng tiêu đề Điều khoản dịch vụ (Terms of Service), nhìn dòng Phiên bản quốc gia (Country version) — đây là quốc gia chính thức mà Google đang gán cho tài khoản của bạn.
• Khách hàng cần cung cấp Mail|Pass|2FA (bắt buộc)
• Admin family add được 5 slot
- GeminiPro + 5TB + 1000 crd flow
- Full model anti: neon, hydrogen, opus 5.5,...
• Gmail phải là tài khoản trắng, chưa tham gia gói Google One / GeminiPro / Pixel nào, không ở trong family, và cần được chuẩn bị theo hướng dẫn.',
    1
),
(
    'GEMINI_PRO_6M_PIXEL',
    'Gemini Pro Your Mail 06 Tháng Pixel (NW) KBH',
    6,
    75000,
    150,
    42,
    '• Trả đơn trong 1-6h hoặc sớm hơn, vui lòng không hối thúc.
- Chỉ nhận Gmail Việt Nam chính chủ, tài khoản trắng chưa tham gia Google One / Family.
• Khách hàng cần cung cấp Mail|Pass|2FA (bắt buộc)
• Admin family add được 5 slot
- GeminiPro + 2TB/5TB + 1000 crd flow
• Cần đóng Hồ sơ thanh toán (HSTT) cũ & bật xác minh 2 bước (2FA 32 ký tự) trước khi gửi đơn.',
    2
)
ON CONFLICT (code) DO NOTHING;
