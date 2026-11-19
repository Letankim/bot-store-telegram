use std::{collections::HashSet, sync::Arc};

use sqlx::PgPool;
use teloxide::Bot;
use tokio::sync::RwLock;

use crate::{
    config::Config,
    crypto::CryptoService,
    services::{activation_service::ActivationService, payos_service::PayOsService},
};

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub pool: PgPool,
    pub bot: Bot,
    pub config: Config,
    pub crypto: CryptoService,
    pub payos_service: PayOsService,
    pub activation_service: ActivationService,
    /// Tracks Telegram users currently entering a custom deposit amount.
    pub awaiting_deposit_amount: RwLock<HashSet<i64>>,
}

impl AppState {
    pub fn new(pool: PgPool, bot: Bot, config: Config) -> SharedState {
        let crypto = CryptoService::new(config.credentials_master_key);
        let payos_service = PayOsService::new(pool.clone(), config.clone());
        let activation_service = ActivationService::new(pool.clone(), crypto.clone());

        Arc::new(Self {
            pool,
            bot,
            config,
            crypto,
            payos_service,
            activation_service,
            awaiting_deposit_amount: RwLock::new(HashSet::new()),
        })
    }
}
