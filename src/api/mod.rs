pub mod admin;
pub mod health;
pub mod middleware;
pub mod web_store;
pub mod webapp;
pub mod webhooks;

use axum::{
    Router, middleware as axum_mw,
    routing::{get, post, put},
};
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};

use crate::state::SharedState;

pub fn build_router(state: SharedState) -> Router {
    let admin_routes = Router::new()
        .route("/settings", get(admin::get_settings).put(admin::update_settings))
        .route("/smm/info", get(admin::smm_partner_info))
        .route("/bot/status", get(admin::bot_status))
        .route("/bot/setup", post(admin::bot_setup))
        .route("/plans", get(admin::list_plans).post(admin::create_plan))
        .route("/orders", get(admin::list_orders))
        .route("/orders/batch-push-api", post(admin::batch_push_orders_to_api))
        .route("/orders/{order_id}/push-api", post(admin::push_order_to_api))
        .route("/orders/{order_id}/sync-link", post(admin::sync_and_resend_order_link))
        .route("/orders/{order_id}/complete", post(admin::complete_order))
        .route("/orders/{order_id}/refund", post(admin::refund_order))
        .route("/plans/{plan_id}", put(admin::update_plan))
        .route("/broadcast", post(admin::broadcast_announcement))
        .route("/wallets/{telegram_id}/credit", post(admin::manual_credit_wallet))
        .route("/users", get(admin::list_users))
        .route("/users/{user_id}/wallet/adjust", post(admin::adjust_user_wallet))
        .route("/users/{user_id}/ban", post(admin::toggle_user_ban))
        .route("/users/{user_id}/transactions", get(admin::list_user_transactions))
        .route("/partner-shop/settings", put(admin::update_partner_shop_settings))
        .route("/partner-shop/balance", get(admin::get_partner_shop_balance))
        .route("/partner-shop/sync", post(admin::sync_partner_shop_now))
        .route("/partner-shop/products", get(admin::list_partner_shop_products))
        .route("/partner-shop/products/{product_id}", put(admin::update_partner_shop_product))
        .route("/partner-shop/orders", get(admin::list_partner_shop_orders))
        .route("/partner-shop/ingest-message", post(admin::ingest_partner_message))
        .route_layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::require_admin_api_key,
        ));

    let web_auth_routes = Router::new()
        .route("/me", get(web_store::web_me))
        .route("/orders", get(web_store::web_list_orders).post(web_store::web_create_order))
        .route("/partner-orders", get(web_store::web_list_partner_orders).post(web_store::web_buy_partner_product))
        .route("/deposits", post(web_store::web_create_deposit))
        .route("/deposits/{order_code}/check", post(web_store::web_check_deposit))
        .route("/deposits/{order_code}/cancel", post(web_store::web_cancel_deposit))
        .route("/wallet/transactions", get(web_store::web_wallet_transactions))
        .route_layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::require_web_session,
        ));

    let api_v1 = Router::new()
        .route("/health", get(health::health_check))
        .route("/webhooks/telegram", post(webhooks::telegram_webhook))
        .route("/webhooks/telegram-admin", post(webhooks::telegram_admin_webhook))
        .route("/webhooks/payos", post(webhooks::payos_webhook))
        .route("/webapp/me", get(webapp::webapp_me))
        .route("/webapp/activations", post(webapp::submit_activation))
        .route("/web/catalog", get(web_store::get_catalog))
        .route("/web/auth/register", post(web_store::web_register))
        .route("/web/auth/login", post(web_store::web_login))
        .route("/web/auth/logout", post(web_store::web_logout))
        .nest("/web", web_auth_routes)
        .route("/admin/login", post(admin::admin_login))
        .route("/admin/logout", post(admin::admin_logout))
        .nest("/admin", admin_routes);

    Router::new()
        .route("/", get(web_store::serve_store_page))
        .route("/admin", get(admin::serve_admin_page))
        .route("/webapp/activate", get(webapp::serve_activate_page))
        .route("/webapp/guide", get(webapp::serve_guide_page))
        .nest("/api/v1", api_v1)
        .layer(axum_mw::from_fn(middleware::security_headers_middleware))
        .layer(axum_mw::from_fn(middleware::request_context_middleware))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1MB max body
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
