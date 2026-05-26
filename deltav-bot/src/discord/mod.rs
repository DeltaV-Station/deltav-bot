use std::sync::Arc;

use poise::serenity_prelude::{self as serenity, GatewayIntents};
use sqlx::{Pool, Sqlite};
use tokio::{
    sync::{Mutex, mpsc::Receiver},
    task::JoinHandle,
};
use tracing::{error, info};

use crate::{
    discord::{
        content_review::{
            component_events::cr_component_task, cr, data::config::CrConfig,
            github_events::cr_github_task, timers::cr_timers_task,
        },
        permissions::{
            data::{PermissionFlags, Permissions},
            perms,
        },
    },
    github::{GitHub, GitHubMessage},
};

mod content_review;
mod permissions;

const EMBED_DESC_MAX_LEN: usize = 4096;

struct Data {
    gh: Arc<GitHub>,
    db: Pool<Sqlite>,
    permissions: Permissions,
    // TODO: need to use the receiver in the event handler, which receives a read-only ref. there's probably a more sane way to do this, but it works for now.
    gh_receiver: Arc<Mutex<Receiver<GitHubMessage>>>,
    cr_config: CrConfig,
}
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;
type ApplicationContext<'a> = poise::ApplicationContext<'a, Data, Error>;

pub async fn initialize(
    token: String,
    github: GitHub,
    db: Pool<Sqlite>,
    receiver: Receiver<GitHubMessage>,
) -> Result<JoinHandle<()>, ()> {
    let permissions = Permissions::new(db.clone());
    if let Ok(operator_id) = std::env::var("DISCORD_OPERATOR_USERID") {
        if let Ok(operator_id) = operator_id.parse::<u64>() {
            info!(
                "Operator is specified in DISCORD_OPERATOR_USERID, granting all flags to {operator_id}."
            );
            permissions
                .set_flags(operator_id, PermissionFlags::all())
                .await
                .map_err(|_| ())?;
        } else {
            error!("Failed to parse DISCORD_OPERATOR_USERID, must be u64.")
        }
    }

    info!("Initializing framework.");
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![cr(), perms()],
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },
            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    gh: Arc::new(github),
                    permissions: Permissions::new(db.clone()),
                    cr_config: CrConfig::from_db(db.clone()).await.unwrap(),
                    db,
                    gh_receiver: Arc::new(Mutex::new(receiver)),
                })
            })
        })
        .build();

    let intents = GatewayIntents::GUILD_MESSAGES;
    let mut client = match serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await
    {
        Ok(x) => x,
        Err(e) => {
            error!("Failed to build client: {e:#?}");
            return Err(());
        }
    };

    info!("Spawning Discord bot task.");
    Ok(tokio::spawn(async move {
        info!("Starting client");
        if let Err(e) = client.start().await {
            error!("Discord client failed: {e:#?}");
        }
    }))
}

async fn event_handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    _framework: poise::FrameworkContext<'_, Data, Error>,
    data: &Data,
) -> Result<(), Error> {
    match event {
        serenity::FullEvent::Ready { data_about_bot } => {
            info!("Logged in as '{}'", data_about_bot.user.name);

            let guild_ids: Vec<u64> = data_about_bot.guilds.iter().map(|x| x.id.get()).collect();
            info!("Present in {} guilds: {:?}", guild_ids.len(), guild_ids);

            tokio::spawn(cr_github_task(
                ctx.clone(),
                data.gh_receiver.clone(),
                data.db.clone(),
                data.gh.clone(),
                data.cr_config.clone(),
            ));

            tokio::spawn(cr_component_task(
                ctx.clone(),
                data.db.clone(),
                data.gh.clone(),
                data.permissions.clone(),
                data.cr_config.clone(),
            ));

            tokio::spawn(cr_timers_task(
                ctx.clone(),
                data.db.clone(),
                data.cr_config.clone(),
            ));
        }
        _ => {}
    }
    Ok(())
}

/// All this does is remove the ` character so reviewers can't close the code block containing their comment
pub fn sanitize_comment(comment: impl AsRef<str>) -> String {
    comment.as_ref().replace('`', "")
}
