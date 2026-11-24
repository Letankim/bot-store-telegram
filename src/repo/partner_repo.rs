use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    domain::partner_shop::{PartnerOrder, PartnerProduct},
    error::AppError,
};

pub async fn list_active_products(pool: &PgPool) -> Result<Vec<PartnerProduct>, AppError> {
    let rows = sqlx::query_as::<_, PartnerProduct>(
        "SELECT * FROM partner_products
         WHERE is_active = TRUE
         ORDER BY category_sort ASC, COALESCE(category_name, '') ASC, external_id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_all_products(pool: &PgPool) -> Result<Vec<PartnerProduct>, AppError> {
    let rows = sqlx::query_as::<_, PartnerProduct>(
        "SELECT * FROM partner_products
         ORDER BY category_sort ASC, COALESCE(category_name, '') ASC, external_id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn find_product_by_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<PartnerProduct>, AppError> {
    let row = sqlx::query_as::<_, PartnerProduct>("SELECT * FROM partner_products WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn find_product_by_external_id(
    pool: &PgPool,
    external_id: i64,
) -> Result<Option<PartnerProduct>, AppError> {
    let row = sqlx::query_as::<_, PartnerProduct>(
        "SELECT * FROM partner_products WHERE external_id = $1",
    )
    .bind(external_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn count_products(pool: &PgPool) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM partner_products")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_synced_product(
    pool: &PgPool,
    external_id: i64,
    name: &str,
    description: &str,
    product_type: &str,
    emoji: &str,
    image_url: Option<&str>,
    category_name: Option<&str>,
    category_emoji: Option<&str>,
    category_sort: i32,
    partner_price_vnd: i64,
    default_markup_vnd: i64,
    stock: i32,
) -> Result<PartnerProduct, AppError> {
    let sell_price_default = partner_price_vnd.saturating_add(default_markup_vnd);
    let row = sqlx::query_as::<_, PartnerProduct>(
        "INSERT INTO partner_products (
            external_id, name, description, product_type, emoji, image_url,
            category_name, category_emoji, category_sort,
            partner_price_vnd, sell_price_vnd, stock, last_synced_at
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NOW())
         ON CONFLICT (external_id) DO UPDATE SET
            name = EXCLUDED.name,
            description = EXCLUDED.description,
            product_type = EXCLUDED.product_type,
            emoji = EXCLUDED.emoji,
            image_url = EXCLUDED.image_url,
            category_name = EXCLUDED.category_name,
            category_emoji = EXCLUDED.category_emoji,
            category_sort = EXCLUDED.category_sort,
            partner_price_vnd = EXCLUDED.partner_price_vnd,
            sell_price_vnd = EXCLUDED.partner_price_vnd + COALESCE(partner_products.custom_markup_vnd, $13),
            stock = EXCLUDED.stock,
            last_synced_at = NOW(),
            updated_at = NOW()
         RETURNING *",
    )
    .bind(external_id)
    .bind(name.trim())
    .bind(description.trim())
    .bind(product_type.trim())
    .bind(emoji.trim())
    .bind(image_url)
    .bind(category_name)
    .bind(category_emoji)
    .bind(category_sort)
    .bind(partner_price_vnd)
    .bind(sell_price_default)
    .bind(stock.max(0))
    .bind(default_markup_vnd)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Admin updates custom profit markup, visibility, or maintenance status of a synced partner product.
#[allow(clippy::too_many_arguments)]
pub async fn update_product_admin(
    pool: &PgPool,
    product_id: Uuid,
    default_markup_vnd: i64,
    set_custom_markup: bool,
    custom_markup_vnd: Option<i64>,
    is_active: Option<bool>,
    is_maintenance: Option<bool>,
    maintenance_note: Option<&str>,
) -> Result<PartnerProduct, AppError> {
    if let Some(m) = custom_markup_vnd {
        if m < 0 {
            return Err(AppError::Validation(
                "Giá lời riêng của sản phẩm không được âm.".to_string(),
            ));
        }
    }

    let updated = sqlx::query_as::<_, PartnerProduct>(
        "UPDATE partner_products
         SET custom_markup_vnd = CASE WHEN $1 THEN $2 ELSE custom_markup_vnd END,
             sell_price_vnd = partner_price_vnd + COALESCE(
                 CASE WHEN $1 THEN $2 ELSE custom_markup_vnd END,
                 $3
             ),
             is_active = COALESCE($4, is_active),
             is_maintenance = COALESCE($5, is_maintenance),
             maintenance_note = COALESCE($6, maintenance_note),
             updated_at = NOW()
         WHERE id = $7
         RETURNING *",
    )
    .bind(set_custom_markup)
    .bind(custom_markup_vnd)
    .bind(default_markup_vnd)
    .bind(is_active)
    .bind(is_maintenance)
    .bind(maintenance_note.map(str::trim))
    .bind(product_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Không tìm thấy sản phẩm đối tác.".to_string()))?;

    Ok(updated)
}

/// Atomically lock & reserve stock on a partner product inside a purchase transaction.
pub async fn reserve_stock_tx(
    tx: &mut Transaction<'_, Postgres>,
    product_id: Uuid,
    quantity: i32,
) -> Result<PartnerProduct, AppError> {
    let prod = sqlx::query_as::<_, PartnerProduct>(
        "SELECT * FROM partner_products WHERE id = $1 FOR UPDATE",
    )
    .bind(product_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Sản phẩm không còn tồn tại.".to_string()))?;

    if !prod.is_active {
        return Err(AppError::NotFound("Sản phẩm hiện đang tạm ngừng bán.".to_string()));
    }
    if prod.is_maintenance {
        let note = if prod.maintenance_note.trim().is_empty() {
            "Vui lòng thử lại sau."
        } else {
            prod.maintenance_note.trim()
        };
        return Err(AppError::Maintenance(format!(
            "{} đang bảo trì. {}",
            prod.name, note
        )));
    }
    if prod.stock < quantity {
        return Err(AppError::OutOfStock);
    }

    let updated = sqlx::query_as::<_, PartnerProduct>(
        "UPDATE partner_products
         SET stock = stock - $1,
             updated_at = NOW()
         WHERE id = $2
         RETURNING *",
    )
    .bind(quantity)
    .bind(product_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(updated)
}

pub async fn create_order_tx(
    tx: &mut Transaction<'_, Postgres>,
    order_code: &str,
    user_id: Uuid,
    product: &PartnerProduct,
    quantity: i32,
) -> Result<PartnerOrder, AppError> {
    let total_partner_cost = product.partner_price_vnd.saturating_mul(i64::from(quantity));
    let total_sell_price = product.sell_price_vnd.saturating_mul(i64::from(quantity));
    let profit = total_sell_price.saturating_sub(total_partner_cost);

    let order = sqlx::query_as::<_, PartnerOrder>(
        "INSERT INTO partner_orders (
            order_code, user_id, partner_product_id, external_product_id, product_name,
            quantity, unit_partner_price_vnd, unit_sell_price_vnd,
            total_partner_cost_vnd, total_sell_price_vnd, profit_vnd, status
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'PROCESSING')
         RETURNING *",
    )
    .bind(order_code)
    .bind(user_id)
    .bind(product.id)
    .bind(product.external_id)
    .bind(&product.name)
    .bind(quantity)
    .bind(product.partner_price_vnd)
    .bind(product.sell_price_vnd)
    .bind(total_partner_cost)
    .bind(total_sell_price)
    .bind(profit)
    .fetch_one(&mut **tx)
    .await?;

    Ok(order)
}

pub async fn mark_order_completed(
    pool: &PgPool,
    order_id: Uuid,
    partner_product_id: Uuid,
    quantity: i32,
    items_encrypted: &[u8],
    nonce: &[u8],
) -> Result<PartnerOrder, AppError> {
    let mut tx = pool.begin().await?;
    let order = sqlx::query_as::<_, PartnerOrder>(
        "UPDATE partner_orders
         SET status = 'COMPLETED',
             items_encrypted = $1,
             nonce = $2,
             updated_at = NOW()
         WHERE id = $3
         RETURNING *",
    )
    .bind(items_encrypted)
    .bind(nonce)
    .bind(order_id)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE partner_products
         SET sold_count = sold_count + $1,
             updated_at = NOW()
         WHERE id = $2",
    )
    .bind(quantity)
    .bind(partner_product_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(order)
}

pub async fn mark_order_refunded_tx(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    partner_product_id: Uuid,
    quantity: i32,
    error_note: &str,
) -> Result<PartnerOrder, AppError> {
    let order = sqlx::query_as::<_, PartnerOrder>(
        "UPDATE partner_orders
         SET status = 'REFUNDED',
             error_note = $1,
             updated_at = NOW()
         WHERE id = $2
         RETURNING *",
    )
    .bind(error_note)
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        "UPDATE partner_products
         SET stock = stock + $1,
             updated_at = NOW()
         WHERE id = $2",
    )
    .bind(quantity)
    .bind(partner_product_id)
    .execute(&mut **tx)
    .await?;

    Ok(order)
}

pub async fn list_orders_by_user(
    pool: &PgPool,
    user_id: Uuid,
    limit: i64,
) -> Result<Vec<PartnerOrder>, AppError> {
    let rows = sqlx::query_as::<_, PartnerOrder>(
        "SELECT * FROM partner_orders
         WHERE user_id = $1
         ORDER BY created_at DESC
         LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_orders_for_admin(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<(Vec<PartnerOrder>, i64), AppError> {
    let rows = sqlx::query_as::<_, PartnerOrder>(
        "SELECT * FROM partner_orders
         ORDER BY created_at DESC
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM partner_orders")
        .fetch_one(pool)
        .await?;

    Ok((rows, total.0))
}
