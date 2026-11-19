use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PayOsDeposit {
    pub id: Uuid,
    pub order_code: i64,
    pub user_id: Uuid,
    pub amount_vnd: i64,
    pub description: String,
    pub checkout_url: Option<String>,
    pub qr_code: Option<String>,
    pub payment_link_id: Option<String>,
    pub reference: Option<String>,
    pub status: String,
    pub paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayOsCreatePaymentRequest {
    pub order_code: i64,
    pub amount: i64,
    pub description: String,
    pub cancel_url: String,
    pub return_url: String,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
pub struct PayOsCreatePaymentResponse {
    pub code: String,
    pub desc: String,
    pub data: Option<PayOsPaymentLinkData>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PayOsPaymentLinkData {
    pub bin: Option<String>,
    pub account_number: Option<String>,
    pub account_name: Option<String>,
    pub amount: i64,
    pub description: String,
    pub order_code: i64,
    pub payment_link_id: String,
    pub status: Option<String>,
    pub checkout_url: String,
    pub qr_code: String,
}

#[derive(Debug, Deserialize)]
pub struct PayOsWebhookPayload {
    pub code: String,
    pub desc: String,
    pub success: Option<bool>,
    pub data: serde_json::Map<String, serde_json::Value>,
    pub signature: String,
}
