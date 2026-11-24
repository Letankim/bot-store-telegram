use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PartnerProduct {
    pub id: Uuid,
    pub external_id: i64,
    pub name: String,
    pub description: String,
    pub product_type: String,
    pub emoji: String,
    pub image_url: Option<String>,
    pub category_name: Option<String>,
    pub category_emoji: Option<String>,
    pub category_sort: i32,
    pub partner_price_vnd: i64,
    pub custom_markup_vnd: Option<i64>,
    pub sell_price_vnd: i64,
    pub stock: i32,
    pub sold_count: i32,
    pub is_active: bool,
    pub is_maintenance: bool,
    pub maintenance_note: String,
    pub last_synced_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Customer-safe view of a synced Partner Shop product (never exposes partner cost or profit margin).
#[derive(Debug, Clone, Serialize)]
pub struct PublicPartnerProduct {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub product_type: String,
    pub emoji: String,
    pub image_url: Option<String>,
    pub category_name: Option<String>,
    pub category_emoji: Option<String>,
    pub sell_price_vnd: i64,
    pub stock: i32,
    pub sold_count: i32,
    pub is_maintenance: bool,
    pub maintenance_note: String,
}

impl From<PartnerProduct> for PublicPartnerProduct {
    fn from(p: PartnerProduct) -> Self {
        Self {
            id: p.id,
            name: p.name,
            description: p.description,
            product_type: p.product_type,
            emoji: p.emoji,
            image_url: p.image_url,
            category_name: p.category_name,
            category_emoji: p.category_emoji,
            sell_price_vnd: p.sell_price_vnd,
            stock: p.stock,
            sold_count: p.sold_count,
            is_maintenance: p.is_maintenance,
            maintenance_note: p.maintenance_note,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PartnerOrder {
    pub id: Uuid,
    pub order_code: String,
    pub user_id: Uuid,
    pub partner_product_id: Uuid,
    pub external_product_id: i64,
    pub product_name: String,
    pub quantity: i32,
    pub unit_partner_price_vnd: i64,
    pub unit_sell_price_vnd: i64,
    pub total_partner_cost_vnd: i64,
    pub total_sell_price_vnd: i64,
    pub profit_vnd: i64,
    #[serde(skip_serializing)]
    pub items_encrypted: Option<Vec<u8>>,
    #[serde(skip_serializing)]
    pub nonce: Option<Vec<u8>>,
    pub status: String,
    pub error_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
