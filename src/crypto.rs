use std::collections::BTreeMap;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::AppError;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct CryptoService {
    master_key: [u8; 32],
}

impl CryptoService {
    pub fn new(master_key: [u8; 32]) -> Self {
        Self { master_key }
    }

    /// Encrypt sensitive string (e.g. `email | pass | 2fa`) with AES-256-GCM + random 96-bit nonce.
    pub fn encrypt(&self, plaintext: &str) -> Result<(Vec<u8>, Vec<u8>), AppError> {
        let cipher = Aes256Gcm::new_from_slice(&self.master_key)
            .map_err(|_| AppError::Internal("Failed to initialize AES-256-GCM cipher".to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|_| AppError::Internal("Failed to encrypt credentials".to_string()))?;

        Ok((ciphertext, nonce_bytes.to_vec()))
    }

    /// Decrypt ciphertext back to plaintext string using AES-256-GCM.
    pub fn decrypt(&self, ciphertext: &[u8], nonce_bytes: &[u8]) -> Result<String, AppError> {
        if nonce_bytes.len() != 12 {
            return Err(AppError::Internal("Invalid AES-GCM nonce length".to_string()));
        }
        let cipher = Aes256Gcm::new_from_slice(&self.master_key)
            .map_err(|_| AppError::Internal("Failed to initialize AES-256-GCM cipher".to_string()))?;
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| AppError::Internal("Failed to decrypt credentials".to_string()))?;

        String::from_utf8(plaintext)
            .map_err(|_| AppError::Internal("Decrypted credentials are not valid UTF-8".to_string()))
    }
}

/// Constant-time string comparison for secrets & signatures.
pub fn constant_time_eq_str(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// Derive a signed HttpOnly cookie token from `ADMIN_API_KEY` using HMAC-SHA256
/// so the raw API key is never exposed in browser cookies or localStorage.
pub fn compute_admin_session_token(admin_api_key: &str) -> Result<String, AppError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(admin_api_key.as_bytes())
        .map_err(|_| AppError::Internal("Failed to derive admin session token".to_string()))?;
    mac.update(b"bot_store_telegram_admin_session_v1");
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Generate PayOS payment creation signature (HMAC-SHA256).
pub fn sign_payos_create_payment(
    amount: i64,
    cancel_url: &str,
    description: &str,
    order_code: i64,
    return_url: &str,
    checksum_key: &str,
) -> Result<String, AppError> {
    let data = format!(
        "amount={amount}&cancelUrl={cancel_url}&description={description}&orderCode={order_code}&returnUrl={return_url}"
    );
    let mut mac = <HmacSha256 as Mac>::new_from_slice(checksum_key.as_bytes())
        .map_err(|_| AppError::Internal("Invalid PayOS checksum key".to_string()))?;
    mac.update(data.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Verify PayOS webhook signature by sorting `data` keys alphabetically and comparing HMAC-SHA256.
pub fn verify_payos_webhook_signature(
    data: &serde_json::Map<String, serde_json::Value>,
    signature: &str,
    checksum_key: &str,
) -> Result<bool, AppError> {
    let mut sorted: BTreeMap<&str, String> = BTreeMap::new();
    for (k, v) in data {
        let val_str = match v {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            other => other.to_string(),
        };
        sorted.insert(k.as_str(), val_str);
    }

    let query_string = sorted
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let mut mac = <HmacSha256 as Mac>::new_from_slice(checksum_key.as_bytes())
        .map_err(|_| AppError::Internal("Invalid PayOS checksum key".to_string()))?;
    mac.update(query_string.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    Ok(constant_time_eq_str(&expected, &signature.to_lowercase()))
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramWebAppUser {
    pub id: i64,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

/// Validate `Telegram.WebApp.initData` using HMAC-SHA256 with `BOT_TOKEN` per Telegram official spec.
pub fn verify_telegram_webapp_init_data(
    init_data: &str,
    bot_token: &str,
    max_age_secs: i64,
) -> Result<TelegramWebAppUser, AppError> {
    let mut pairs: BTreeMap<String, String> = BTreeMap::new();
    let mut received_hash: Option<String> = None;

    for (k, v) in url::form_urlencoded::parse(init_data.as_bytes()) {
        if k == "hash" {
            received_hash = Some(v.into_owned());
        } else {
            pairs.insert(k.into_owned(), v.into_owned());
        }
    }

    let received_hash = received_hash
        .ok_or_else(|| AppError::Unauthorized("Thiếu chữ ký hash trong Telegram initData".to_string()))?;

    // Check auth_date freshness
    let auth_date = pairs
        .get("auth_date")
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| AppError::Unauthorized("Thiếu hoặc sai auth_date trong initData".to_string()))?;

    let now = Utc::now().timestamp();
    if (now - auth_date).abs() > max_age_secs {
        return Err(AppError::Unauthorized(
            "Phiên Mini App đã hết hạn, vui lòng đóng và mở lại từ Bot.".to_string(),
        ));
    }

    let data_check_string = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    // secret_key = HMAC_SHA256(key = "WebAppData", msg = bot_token)
    let mut secret_mac = <HmacSha256 as Mac>::new_from_slice(b"WebAppData")
        .map_err(|_| AppError::Internal("HMAC init failed".to_string()))?;
    secret_mac.update(bot_token.as_bytes());
    let secret_key = secret_mac.finalize().into_bytes();

    // computed_hash = HEX(HMAC_SHA256(key = secret_key, msg = data_check_string))
    let mut check_mac = <HmacSha256 as Mac>::new_from_slice(&secret_key)
        .map_err(|_| AppError::Internal("HMAC init failed".to_string()))?;
    check_mac.update(data_check_string.as_bytes());
    let computed_hash = hex::encode(check_mac.finalize().into_bytes());

    if !constant_time_eq_str(&computed_hash, &received_hash.to_lowercase()) {
        return Err(AppError::Unauthorized(
            "Chữ ký Telegram WebApp không hợp lệ.".to_string(),
        ));
    }

    let user_json = pairs
        .get("user")
        .ok_or_else(|| AppError::Unauthorized("Thiếu thông tin user trong initData".to_string()))?;

    serde_json::from_str::<TelegramWebAppUser>(user_json)
        .map_err(|_| AppError::Unauthorized("Không thể đọc thông tin user từ initData".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aes_256_gcm_roundtrip() {
        let key = [0x42u8; 32];
        let crypto = CryptoService::new(key);
        let secret = "customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl";
        let (ct, nonce) = crypto.encrypt(secret).unwrap();
        assert_ne!(ct, secret.as_bytes());
        let decrypted = crypto.decrypt(&ct, &nonce).unwrap();
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn test_payos_webhook_signature_verification() {
        let checksum_key = "test_checksum_key";
        let mut data = serde_json::Map::new();
        data.insert("orderCode".to_string(), serde_json::json!(123456));
        data.insert("amount".to_string(), serde_json::json!(40000));
        data.insert("description".to_string(), serde_json::json!("NAP 123456"));

        let query = "amount=40000&description=NAP 123456&orderCode=123456";
        let mut mac = <HmacSha256 as Mac>::new_from_slice(checksum_key.as_bytes()).unwrap();
        mac.update(query.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());

        assert!(verify_payos_webhook_signature(&data, &sig, checksum_key).unwrap());
        assert!(!verify_payos_webhook_signature(&data, "deadbeef", checksum_key).unwrap());
    }
}
