use std::env;

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotMode {
    Polling,
    Webhook,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub app_env: String,
    pub host: String,
    pub port: u16,
    pub public_base_url: String,
    pub database_url: String,
    pub db_max_connections: u32,
    pub telegram_bot_token: String,
    pub bot_mode: BotMode,
    pub telegram_webhook_secret: String,
    pub admin_telegram_id: Option<i64>,
    pub support_handle: String,
    pub credentials_master_key: [u8; 32],
    pub admin_api_key: String,
    pub payos_client_id: String,
    pub payos_api_key: String,
    pub payos_checksum_key: String,
    pub payos_base_url: String,
    pub referral_reward_vnd: i64,
}

impl Config {
    pub fn from_env() -> Result<Self, AppError> {
        let _ = dotenvy::dotenv();

        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "10000".to_string())
            .parse::<u16>()
            .map_err(|_| AppError::Config("PORT must be a valid u16".to_string()))?;

        let public_base_url = env::var("PUBLIC_BASE_URL")
            .unwrap_or_else(|_| format!("http://localhost:{port}"))
            .trim_end_matches('/')
            .to_string();

        let database_url = required_var("DATABASE_URL")?;
        let db_max_connections = env::var("DB_MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u32>()
            .map_err(|_| AppError::Config("DB_MAX_CONNECTIONS must be a valid u32".to_string()))?;

        let telegram_bot_token = required_var("TELEGRAM_BOT_TOKEN")?;
        let bot_mode = match env::var("BOT_MODE")
            .unwrap_or_else(|_| "polling".to_string())
            .to_lowercase()
            .as_str()
        {
            "webhook" => BotMode::Webhook,
            _ => BotMode::Polling,
        };
        let telegram_webhook_secret = env::var("TELEGRAM_WEBHOOK_SECRET")
            .unwrap_or_else(|_| "dev_telegram_webhook_secret_token".to_string());

        let admin_telegram_id = env::var("ADMIN_TELEGRAM_ID")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .and_then(|v| v.trim().parse::<i64>().ok());

        let support_handle = env::var("SUPPORT_HANDLE").unwrap_or_else(|_| "@kim12ad".to_string());

        let master_key_hex = required_var("CREDENTIALS_MASTER_KEY")?;
        let master_key_bytes = hex::decode(master_key_hex.trim()).map_err(|_| {
            AppError::Config("CREDENTIALS_MASTER_KEY must be valid 64-char hex (32 bytes)".to_string())
        })?;
        let credentials_master_key: [u8; 32] = master_key_bytes.try_into().map_err(|_| {
            AppError::Config("CREDENTIALS_MASTER_KEY must be exactly 32 bytes (64 hex chars)".to_string())
        })?;

        let admin_api_key = required_var("ADMIN_API_KEY")?;

        let payos_client_id = env::var("PAYOS_CLIENT_ID").unwrap_or_default();
        let payos_api_key = env::var("PAYOS_API_KEY").unwrap_or_default();
        let payos_checksum_key = env::var("PAYOS_CHECKSUM_KEY").unwrap_or_default();
        let payos_base_url = env::var("PAYOS_BASE_URL")
            .unwrap_or_else(|_| "https://api-merchant.payos.vn".to_string())
            .trim_end_matches('/')
            .to_string();

        let referral_reward_vnd = env::var("REFERRAL_REWARD_VND")
            .unwrap_or_else(|_| "5000".to_string())
            .parse::<i64>()
            .unwrap_or(5000);

        Ok(Self {
            app_env,
            host,
            port,
            public_base_url,
            database_url,
            db_max_connections,
            telegram_bot_token,
            bot_mode,
            telegram_webhook_secret,
            admin_telegram_id,
            support_handle,
            credentials_master_key,
            admin_api_key,
            payos_client_id,
            payos_api_key,
            payos_checksum_key,
            payos_base_url,
            referral_reward_vnd,
        })
    }

    pub fn guide_url(&self, plan_id: Option<&str>) -> String {
        match plan_id {
            Some(pid) => format!("{}/webapp/guide?plan_id={}", self.public_base_url, pid),
            None => format!("{}/webapp/guide", self.public_base_url),
        }
    }

    pub fn activate_url(&self, plan_id: &str) -> String {
        format!("{}/webapp/activate?plan_id={}", self.public_base_url, plan_id)
    }
}

fn required_var(name: &str) -> Result<String, AppError> {
    env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Config(format!("Missing required environment variable: {name}")))
}
