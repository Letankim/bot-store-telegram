-- Sanitize any existing third-party Telegram @usernames or t.me/ links in products
-- so customers are NEVER redirected to external sellers (always points to @kim12ad / https://t.me/kim12ad).

UPDATE gemini_plans
SET
    name = regexp_replace(
        regexp_replace(
            name,
            '(https?://)?(www\.)?(t\.me|telegram\.me|telegram\.dog)/[A-Za-z0-9_/+?=&.%~#-]+',
            'https://t.me/kim12ad',
            'gi'
        ),
        '(^|[^A-Za-z0-9._%+-])@[A-Za-z][A-Za-z0-9_]{2,}(?![.][A-Za-z]{2,})',
        '\1@kim12ad',
        'gi'
    ),
    description_note = regexp_replace(
        regexp_replace(
            description_note,
            '(https?://)?(www\.)?(t\.me|telegram\.me|telegram\.dog)/[A-Za-z0-9_/+?=&.%~#-]+',
            'https://t.me/kim12ad',
            'gi'
        ),
        '(^|[^A-Za-z0-9._%+-])@[A-Za-z][A-Za-z0-9_]{2,}(?![.][A-Za-z]{2,})',
        '\1@kim12ad',
        'gi'
    ),
    maintenance_note = regexp_replace(
        regexp_replace(
            maintenance_note,
            '(https?://)?(www\.)?(t\.me|telegram\.me|telegram\.dog)/[A-Za-z0-9_/+?=&.%~#-]+',
            'https://t.me/kim12ad',
            'gi'
        ),
        '(^|[^A-Za-z0-9._%+-])@[A-Za-z][A-Za-z0-9_]{2,}(?![.][A-Za-z]{2,})',
        '\1@kim12ad',
        'gi'
    ),
    updated_at = NOW();

UPDATE partner_products
SET
    name = regexp_replace(
        regexp_replace(
            name,
            '(https?://)?(www\.)?(t\.me|telegram\.me|telegram\.dog)/[A-Za-z0-9_/+?=&.%~#-]+',
            'https://t.me/kim12ad',
            'gi'
        ),
        '(^|[^A-Za-z0-9._%+-])@[A-Za-z][A-Za-z0-9_]{2,}(?![.][A-Za-z]{2,})',
        '\1@kim12ad',
        'gi'
    ),
    description = regexp_replace(
        regexp_replace(
            description,
            '(https?://)?(www\.)?(t\.me|telegram\.me|telegram\.dog)/[A-Za-z0-9_/+?=&.%~#-]+',
            'https://t.me/kim12ad',
            'gi'
        ),
        '(^|[^A-Za-z0-9._%+-])@[A-Za-z][A-Za-z0-9_]{2,}(?![.][A-Za-z]{2,})',
        '\1@kim12ad',
        'gi'
    ),
    maintenance_note = regexp_replace(
        regexp_replace(
            maintenance_note,
            '(https?://)?(www\.)?(t\.me|telegram\.me|telegram\.dog)/[A-Za-z0-9_/+?=&.%~#-]+',
            'https://t.me/kim12ad',
            'gi'
        ),
        '(^|[^A-Za-z0-9._%+-])@[A-Za-z][A-Za-z0-9_]{2,}(?![.][A-Za-z]{2,})',
        '\1@kim12ad',
        'gi'
    ),
    updated_at = NOW();
