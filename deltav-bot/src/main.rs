#![allow(dead_code)] // TODO: remove

use std::str::FromStr;

use jsonwebtoken::EncodingKey;
use octocrab::models::AppId;
use tracing::{error, info};

use crate::github::{GhAppConfig, GitHub};

mod consts;
mod discord;
mod github;

macro_rules! required_env {
    ($variable_name: expr, $local_var: ident) => {
        let val: &str = $variable_name;
        let Ok($local_var) = std::env::var(val) else {
            tracing::error!("[FATAL] Missing environment variable: {}", val);
            return;
        };
    };
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    required_env!("GH_REPO_OWNER", repo_owner);
    required_env!("GH_REPO_NAME", repo_name);
    required_env!("GH_APP_KEY", app_key);
    let Ok(app_key) = EncodingKey::from_rsa_pem(app_key.as_bytes()) else {
        error!("[FATAL] Invalid GitHub Application Key. Must be an RSA key in PEM format.");
        return;
    };
    required_env!("GH_APP_ID", app_id);
    let Ok(app_id) = app_id.parse::<u64>() else {
        error!("[FATAL] Invalid GitHub Application ID. Must be u64.");
        return;
    };

    required_env!("WEBHOOK_PORT", webhook_port);
    let Ok(webhook_port) = webhook_port.parse::<u16>() else {
        error!("[FATAL] Invalid Webhook port. Must be u16.");
        return;
    };
    required_env!("WEBHOOK_SECRET", webhook_secret);
    if webhook_secret.len() < 16 {
        error!("[FATAL] The Webhook secret must be at least 16 characters long.");
        return;
    }

    required_env!("DISCORD_TOKEN", discord_token);

    required_env!("DATABASE_URL", database_url);
    let sqlite_opts = match sqlx::sqlite::SqliteConnectOptions::from_str(&database_url) {
        Ok(x) => x,
        Err(e) => {
            error!("[FATAL] Failed to parse DATABASE_URL: {e:#?}");
            return;
        }
    };

    info!("Setting up database.");
    let db = match sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(sqlite_opts.create_if_missing(true))
        .await
    {
        Ok(x) => x,
        Err(e) => {
            error!("[FATAL] Failed to set up Database: {e:#?}");
            return;
        }
    };

    info!("Running migrations.");
    if let Err(e) = sqlx::migrate!("./migrations").run(&db).await {
        error!("[FATAL] Failed to run database migrations: {e:#?}");
        return;
    }

    let Ok((hook, gh)) = GitHub::initialize(
        webhook_port,
        webhook_secret,
        GhAppConfig {
            id: AppId(app_id),
            key: app_key,
            repo_owner,
            repo_name,
        },
    )
    .await
    else {
        error!("[FATAL] GitHub integration failed to initialize.");
        return;
    };

    let Ok(bot_thread) = discord::initialize(discord_token, gh, db, hook.receiver).await else {
        error!("[FATAL] Discord bot failed to initialize.");
        return;
    };

    tokio::select! {
       _ = hook.thread => {
           info!("GitHub webhook shut down. Exiting.")
       }
       _ = bot_thread => {
           info!("Discord bot shut down. Exiting.")
       }
    }
}
