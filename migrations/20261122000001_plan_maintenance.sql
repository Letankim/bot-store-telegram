ALTER TABLE gemini_plans
    ADD COLUMN IF NOT EXISTS is_maintenance BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS maintenance_note TEXT NOT NULL DEFAULT 'Vui lòng thử lại sau.';
