use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use sqlx::PgPool;
use teloxide::Bot;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    config::Config,
    crypto::CryptoService,
    services::{activation_service::ActivationService, payos_service::PayOsService},
};

pub type SharedState = Arc<AppState>;

#[derive(Debug, Clone)]
pub enum AdminBotPendingAction {
    RefundOrder { order_id: Uuid, order_code: String },
    AddPlanStock { plan_id: Uuid, plan_name: String },
    BroadcastCustom,
    SearchUser,
    AdjustUserWallet { user_id: Uuid, user_name: String, sign: i64 },
}

pub struct AppState {
    pub pool: PgPool,
    pub bot: Bot,
    pub admin_bot: Option<Bot>,
    pub config: Config,
    pub crypto: CryptoService,
    pub payos_service: PayOsService,
    pub activation_service: ActivationService,
    /// Tracks Telegram users currently entering a custom deposit amount.
    pub awaiting_deposit_amount: RwLock<HashSet<i64>>,
    /// Tracks Telegram IDs authenticated on the dedicated Admin Bot via `ADMIN_API_KEY`.
    pub admin_bot_sessions: RwLock<HashSet<i64>>,
    /// Tracks multi-step input actions on the dedicated Admin Bot.
    pub admin_bot_pending: RwLock<HashMap<i64, AdminBotPendingAction>>,
}

impl AppState {
    pub fn new(pool: PgPool, bot: Bot, admin_bot: Option<Bot>, config: Config) -> SharedState {
        let crypto = CryptoService::new(config.credentials_master_key);
        let payos_service = PayOsService::new(pool.clone(), config.clone());
        let activation_service = ActivationService::new(pool.clone(), crypto.clone());

        Arc::new(Self {
            pool,
            bot,
            admin_bot,
            config,
            crypto,
            payos_service,
            activation_service,
            awaiting_deposit_amount: RwLock::new(HashSet::new()),
            admin_bot_sessions: RwLock::new(HashSet::new()),
            admin_bot_pending: RwLock::new(HashMap::new()),
        })
    }
}
