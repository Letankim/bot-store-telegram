pub mod admin_bot;
pub mod callbacks;
pub mod commands;
pub mod keyboards;
pub mod messages;

use teloxide::{
    prelude::*,
    types::Update,
    utils::command::BotCommands,
};

use crate::state::SharedState;

/// Register slash commands menu on Telegram (`/start`, `/pixel`, `/payadd`, `/guide`).
pub async fn register_commands(bot: &Bot) {
    if let Err(err) = bot.set_my_commands(commands::Command::bot_commands()).await {
        tracing::warn!(error = ?err, "Failed to register Telegram slash commands");
    }
}

/// Dispatch a single Telegram `Update` (used by both Webhook mode and Polling mode).
pub async fn dispatch_update(bot: Bot, update: Update, state: SharedState) {
    match update.kind {
        teloxide::types::UpdateKind::Message(msg) => {
            if let Err(err) = commands::handle_message(bot, msg, state).await {
                tracing::error!(error = ?err, "Error handling Telegram message");
            }
        }
        teloxide::types::UpdateKind::CallbackQuery(q) => {
            if let Err(err) = callbacks::handle_callback(bot, q, state).await {
                tracing::error!(error = ?err, "Error handling Telegram callback query");
            }
        }
        _ => {}
    }
}

/// Run long-polling loop for local development (`BOT_MODE=polling`).
pub async fn run_polling(state: SharedState) {
    let bot = state.bot.clone();
    register_commands(&bot).await;

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(commands::handle_message))
        .branch(Update::filter_callback_query().endpoint(callbacks::handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .build()
        .dispatch()
        .await;
}
