use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ActivationOrder {
    pub id: Uuid,
    pub order_code: String,
    pub user_id: Uuid,
    pub plan_id: Uuid,
    pub price_paid_vnd: i64,
    pub gmail_masked: String,
    #[serde(skip_serializing)]
    pub credentials_encrypted: Vec<u8>,
    #[serde(skip_serializing)]
    pub nonce: Vec<u8>,
    pub status: String,
    pub admin_note: Option<String>,
    pub external_job_id: Option<i64>,
    pub external_status: Option<String>,
    pub external_step: Option<String>,
    pub external_result: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// Validated & normalized credentials formatted strictly as:
/// `customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl`
/// or with optional 2-3 backup codes (8 digits each):
/// `customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl | 12345678,98765432`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCredentials {
    pub email: String,
    pub password: String,
    pub two_fa_secret: String,
    pub two_fa_compact_upper: String,
    pub backup_codes: Vec<String>,
    pub formatted_line: String,
    pub gmail_masked: String,
}

impl NormalizedCredentials {
    pub fn build(
        email: &str,
        password: &str,
        two_fa_raw: &str,
        backup_codes_raw: Option<&[String]>,
    ) -> Result<Self, AppError> {
        let email_clean = email.trim().to_lowercase();
        if !email_clean.contains('@')
            || !email_clean.ends_with("@gmail.com")
            || email_clean.len() < 12
            || email_clean.contains(' ')
            || email_clean.contains('|')
        {
            return Err(AppError::Validation(
                "Địa chỉ email phải là Gmail hợp lệ (ví dụ: customer1@gmail.com).".to_string(),
            ));
        }

        let password_clean = password.trim().to_string();
        if password_clean.is_empty() || password_clean.contains('|') {
            return Err(AppError::Validation(
                "Mật khẩu Gmail không được để trống và không chứa ký tự '|'.".to_string(),
            ));
        }

        let two_fa_clean: String = two_fa_raw
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect();

        let is_valid_base32 = !two_fa_clean.is_empty()
            && two_fa_clean.len() >= 16
            && two_fa_clean.len() <= 64
            && two_fa_clean
                .chars()
                .all(|c| c.is_ascii_lowercase() || ('2'..='7').contains(&c));

        if !is_valid_base32 {
            return Err(AppError::Validation(
                "Mã 2FA phải là chuỗi khóa thiết lập Base32 (16-32 ký tự chữ a-z và số 2-7, ví dụ: ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl), không phải mã 6 số.".to_string(),
            ));
        }

        let mut backup_codes = Vec::new();
        if let Some(raw_list) = backup_codes_raw {
            for item in raw_list {
                for sub in item.split([',', '\n', ';']) {
                    let trimmed = sub.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    // Google displays 8-digit backup codes as `1234 5678` — strip spaces/hyphens
                    let digits: String = trimmed
                        .chars()
                        .filter(|c| !c.is_whitespace() && *c != '-')
                        .collect();
                    if digits.len() != 8 || !digits.chars().all(|c| c.is_ascii_digit()) {
                        return Err(AppError::Validation(
                            "Mỗi Backup Code phải gồm đúng 8 chữ số (ví dụ: 12345678).".to_string(),
                        ));
                    }
                    backup_codes.push(digits);
                }
            }
        }

        if !backup_codes.is_empty() && !(2..=3).contains(&backup_codes.len()) {
            return Err(AppError::Validation(
                "Nếu nhập Backup Codes, vui lòng nhập từ 2 đến 3 mã (mỗi mã đúng 8 chữ số)."
                    .to_string(),
            ));
        }

        let grouped_2fa = two_fa_clean
            .as_bytes()
            .chunks(4)
            .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
            .collect::<Vec<_>>()
            .join(" ");

        let formatted_line = if backup_codes.is_empty() {
            format!("{email_clean} | {password_clean} | {grouped_2fa}")
        } else {
            format!(
                "{email_clean} | {password_clean} | {grouped_2fa} | {}",
                backup_codes.join(",")
            )
        };
        let gmail_masked = mask_gmail(&email_clean);

        Ok(Self {
            email: email_clean,
            password: password_clean,
            two_fa_secret: grouped_2fa,
            two_fa_compact_upper: two_fa_clean.to_uppercase(),
            backup_codes,
            formatted_line,
            gmail_masked,
        })
    }

    /// Build validated order input according to the product's configured `form_mode`:
    /// - `"mail_pass_2fa"`: strict Gmail | Password | 32-char Base32 2FA (+ optional Backup Codes)
    /// - `"email_only"`: only a valid email address (any domain, e.g. invite Family/Canva/ChatGPT)
    /// - `"custom_note"`: a single custom text input (username, link, note, etc.)
    /// - `"none"`: no form fields required (instant direct purchase)
    pub fn build_for_form_mode(
        form_mode: &str,
        email: &str,
        password: &str,
        two_fa_raw: &str,
        backup_codes_raw: Option<&[String]>,
        custom_input: Option<&str>,
    ) -> Result<Self, AppError> {
        match form_mode {
            "email_only" => {
                let raw = if !email.trim().is_empty() {
                    email.trim()
                } else {
                    custom_input.unwrap_or("").trim()
                };
                let email_clean = raw.to_lowercase();
                let valid = email_clean.len() >= 5
                    && email_clean.len() <= 250
                    && !email_clean.contains(char::is_whitespace)
                    && !email_clean.contains('|')
                    && email_clean
                        .split_once('@')
                        .map(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
                        .unwrap_or(false);
                if !valid {
                    return Err(AppError::Validation(
                        "Vui lòng nhập địa chỉ email hợp lệ (ví dụ: tenban@gmail.com).".to_string(),
                    ));
                }
                let gmail_masked = mask_gmail(&email_clean);
                Ok(Self {
                    email: email_clean.clone(),
                    password: String::new(),
                    two_fa_secret: String::new(),
                    two_fa_compact_upper: String::new(),
                    backup_codes: Vec::new(),
                    formatted_line: email_clean,
                    gmail_masked,
                })
            }
            "custom_note" => {
                let raw = custom_input
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| email.trim());
                if raw.is_empty() || raw.len() > 500 {
                    return Err(AppError::Validation(
                        "Vui lòng nhập thông tin theo yêu cầu (từ 1 đến 500 ký tự).".to_string(),
                    ));
                }
                let preview: String = raw.chars().take(48).collect();
                let gmail_masked = if raw.chars().count() > 48 {
                    format!("{preview}...")
                } else {
                    preview
                };
                Ok(Self {
                    email: String::new(),
                    password: String::new(),
                    two_fa_secret: String::new(),
                    two_fa_compact_upper: String::new(),
                    backup_codes: Vec::new(),
                    formatted_line: raw.to_string(),
                    gmail_masked,
                })
            }
            "none" => Ok(Self {
                email: String::new(),
                password: String::new(),
                two_fa_secret: String::new(),
                two_fa_compact_upper: String::new(),
                backup_codes: Vec::new(),
                formatted_line: "Không yêu cầu thông tin (mua trực tiếp)".to_string(),
                gmail_masked: "Mua trực tiếp".to_string(),
            }),
            _ => Self::build(email, password, two_fa_raw, backup_codes_raw),
        }
    }

    /// Parse back a decrypted line `email | pass | 2fa [| code1,code2]`.
    pub fn parse_formatted(line: &str) -> Result<Self, AppError> {
        let parts: Vec<&str> = line.split('|').map(str::trim).collect();
        if parts.len() < 3 {
            return Err(AppError::Validation(
                "Dữ liệu tài khoản không đúng định dạng email | pass | 2fa".to_string(),
            ));
        }
        // Only parse part[3] as backup codes if it doesn't contain '@' (backward compatible with legacy recovery_email rows)
        let backup_list: Option<Vec<String>> = parts
            .get(3)
            .copied()
            .filter(|s| !s.is_empty() && !s.contains('@'))
            .map(|s| vec![s.to_string()]);
        Self::build(parts[0], parts[1], parts[2], backup_list.as_deref())
    }
}

pub fn mask_gmail(email: &str) -> String {
    let Some((local, domain)) = email.split_once('@') else {
        return "***".to_string();
    };
    let chars: Vec<char> = local.chars().collect();
    if chars.len() <= 3 {
        format!("***@{domain}")
    } else {
        let prefix: String = chars.iter().take(3).collect();
        let suffix: String = chars.iter().skip(chars.len() - 1).collect();
        format!("{prefix}***{suffix}@{domain}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_credentials_exact_format() {
        let codes = vec!["1234 5678".to_string(), "98765432".to_string()];
        let res = NormalizedCredentials::build(
            "  Customer1@gmail.com ",
            " abcxyz ",
            "YDWX-XCPO 4q4q zye5 fdpt ti3w ungl mggl",
            Some(&codes),
        )
        .unwrap();

        assert_eq!(
            res.formatted_line,
            "customer1@gmail.com | abcxyz | ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl | 12345678,98765432"
        );
        assert_eq!(res.two_fa_compact_upper, "YDWXXCPO4Q4QZYE5FDPTTI3WUNGLMGGL");
        assert_eq!(res.backup_codes, vec!["12345678", "98765432"]);
        assert_eq!(res.gmail_masked, "cus***1@gmail.com");
    }

    #[test]
    fn test_reject_invalid_backup_code_count() {
        let one_code = vec!["12345678".to_string()];
        let err = NormalizedCredentials::build(
            "customer1@gmail.com",
            "abcxyz",
            "ydwx xcpo 4q4q zye5 fdpt ti3w ungl mggl",
            Some(&one_code),
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_reject_six_digit_otp() {
        let err = NormalizedCredentials::build("customer1@gmail.com", "abcxyz", "123456", None);
        assert!(err.is_err());
    }

    #[test]
    fn test_dynamic_form_modes() {
        let email_only = NormalizedCredentials::build_for_form_mode(
            "email_only",
            "User@Company.com",
            "",
            "",
            None,
            None,
        )
        .unwrap();
        assert_eq!(email_only.formatted_line, "user@company.com");

        let custom = NormalizedCredentials::build_for_form_mode(
            "custom_note",
            "",
            "",
            "",
            None,
            Some("@my_canva_account"),
        )
        .unwrap();
        assert_eq!(custom.formatted_line, "@my_canva_account");

        let direct =
            NormalizedCredentials::build_for_form_mode("none", "", "", "", None, None).unwrap();
        assert_eq!(direct.gmail_masked, "Mua trực tiếp");
    }
}
