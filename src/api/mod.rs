pub mod admin;
pub mod health;
pub mod middleware;
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
        .route("/orders/{order_id}/complete", post(admin::complete_order))
        .route("/orders/{order_id}/refund", post(admin::refund_order))
        .route("/plans/{plan_id}", put(admin::update_plan))
        .route("/wallets/{telegram_id}/credit", post(admin::manual_credit_wallet))
        .route_layer(axum_mw::from_fn_with_state(
            state.clone(),
            middleware::require_admin_api_key,
        ));

    let api_v1 = Router::new()
        .route("/health", get(health::health_check))
        .route("/webhooks/telegram", post(webhooks::telegram_webhook))
        .route("/webhooks/payos", post(webhooks::payos_webhook))
        .route("/webapp/me", get(webapp::webapp_me))
        .route("/webapp/activations", post(webapp::submit_activation))
        .nest("/admin", admin_routes);

    Router::new()
        .route("/admin", get(admin::serve_admin_page))
        .route("/webapp/activate", get(webapp::serve_activate_page))
        .route("/webapp/guide", get(webapp::serve_guide_page))
        .nest("/api/v1", api_v1)
        .layer(axum_mw::from_fn(middleware::request_context_middleware))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1MB max body
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
