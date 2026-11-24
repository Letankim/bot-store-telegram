use std::{net::SocketAddr, time::Duration};

use sqlx::postgres::PgPoolOptions;
use teloxide::{Bot, requests::Requester, payloads::SetWebhookSetters};
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use url::Url;

use bot_store_telegram::{
    api, bot,
    config::{BotMode, Config},
    services::{partner_shop_service, smm_service},
    state::AppState,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load Configuration
    let config = Config::from_env()?;

    // 2. Initialize Structured Tracing (JSON in production, pretty in dev)
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,bot_store_telegram=debug,sqlx=warn".into());

    if config.app_env == "production" {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    tracing::info!(
        env = %config.app_env,
        bot_mode = ?config.bot_mode,
        "Starting BotStoreTelegram server..."
    );

    // 3. Connect PostgreSQL Pool (Render-optimized)
    let pool = PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .min_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(1800))
        .connect(&config.database_url)
        .await?;

    // 4. Run SQLx Migrations automatically on startup
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("PostgreSQL migrations verified/applied successfully.");

    // 5. Initialize Telegram Bots (Customer Bot + optional dedicated Admin Bot) & Shared State
    let tg_bot = Bot::new(&config.telegram_bot_token);
    let admin_bot = config.admin_bot_token.as_deref().map(Bot::new);
    let state = AppState::new(pool, tg_bot.clone(), admin_bot.clone(), config.clone());

    // 5.5 Start Background FIFO Queue Worker & Partner Shop Periodic Sync Worker
    smm_service::spawn_auto_queue_worker(state.clone());
    partner_shop_service::spawn_partner_shop_sync_worker(state.clone());

    // 6. Configure Webhook or Launch Polling
    if config.bot_mode == BotMode::Webhook {
        bot::register_commands(&tg_bot).await;
        let webhook_url = format!("{}/api/v1/webhooks/telegram", config.public_base_url);
        let parsed_url = Url::parse(&webhook_url)?;
        tg_bot
            .set_webhook(parsed_url)
            .secret_token(config.telegram_webhook_secret.clone())
            .await?;
        tracing::info!(url = %webhook_url, "Registered Telegram Webhook successfully.");

        if let Some(adm_bot) = admin_bot {
            bot::admin_bot::register_commands(&adm_bot).await;
            let admin_wh_url = format!("{}/api/v1/webhooks/telegram-admin", config.public_base_url);
            let parsed_admin_url = Url::parse(&admin_wh_url)?;
            adm_bot
                .set_webhook(parsed_admin_url)
                .secret_token(config.telegram_webhook_secret.clone())
                .await?;
            tracing::info!(url = %admin_wh_url, "Registered Admin Bot Telegram Webhook successfully.");
        }
    } else {
        let polling_state = state.clone();
        tokio::spawn(async move {
            tracing::info!("Running Telegram Bot in long-polling mode...");
            bot::run_polling(polling_state).await;
        });

        if let Some(adm_bot) = admin_bot {
            let adm_state = state.clone();
            tokio::spawn(async move {
                tracing::info!("Running dedicated Admin Bot in long-polling mode...");
                bot::admin_bot::run_polling(adm_bot, adm_state).await;
            });
        }
    }

    // 7. Launch Axum HTTP Server (API + PayOS Webhook + Mini App Form + Guide Page)
    let app = api::build_router(state);
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(address = %addr, "HTTP server listening");

    axum::serve(listener, app).await?;
    Ok(())
}
