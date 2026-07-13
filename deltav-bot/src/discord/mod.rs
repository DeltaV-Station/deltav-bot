use std::sync::Arc;

use poise::serenity_prelude::{self as serenity, GatewayIntents, MessageType};
use sqlx::{Pool, Sqlite};
use tokio::{
    sync::{Mutex, mpsc::Receiver},
    task::JoinHandle,
};
use tracing::{error, info};

use crate::{
    discord::{
        content_review::{
            component_events::cr_component_task,
            cr,
            data::config::CrConfig,
            github_events::cr_github_task,
            raised_issues::{
                cr_issue_dismiss_context, cr_issue_dismiss_override_context,
                cr_issue_override_context, cr_issue_overview_context, cr_issue_raise_context,
                cr_issue_view_context,
            },
            timers::cr_timers_task,
        },
        permissions::{
            data::{PermissionFlags, Permissions},
            perms,
        },
        pr_feeds::{data::PrDashboards, pr_feeds},
    },
    github::{GitHub, GitHubMessage},
};

mod content_review;
mod permissions;
mod pr_feeds;

const EMBED_DESC_MAX_LEN: usize = 4096;

struct Data {
    gh: Arc<GitHub>,
    db: Pool<Sqlite>,
    permissions: Permissions,
    // TODO: need to use the receiver in the event handler, which receives a read-only ref. there's probably a more sane way to do this, but it works for now.
    gh_receiver: Arc<Mutex<Receiver<GitHubMessage>>>,
    cr_config: CrConfig,
    pr_feeds: PrDashboards,
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
            commands: vec![
                cr(),
                perms(),
                pr_feeds(),
                // the following are context menu actions, they can't be subcommands of slash commands
                cr_issue_raise_context(),
                cr_issue_view_context(),
                cr_issue_override_context(),
                cr_issue_dismiss_context(),
                cr_issue_dismiss_override_context(),
                cr_issue_overview_context(),
            ],
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
                    pr_feeds: PrDashboards::from_db(db.clone())
                        .await
                        .expect("Failed initial PR Feeds load"),
                    cr_config: CrConfig::from_db(db.clone())
                        .await
                        .expect("Failed initial CR Config load"),
                    db,
                    gh_receiver: Arc::new(Mutex::new(receiver)),
                })
            })
        })
        .build();

    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
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
    framework: poise::FrameworkContext<'_, Data, Error>,
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
                data.pr_feeds.clone(),
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

        serenity::FullEvent::Message { new_message } => {
            if new_message.author.id != framework.bot_id {
                return Ok(());
            }

            if new_message.kind == MessageType::PinsAdd {
                if let Err(e) = new_message.delete(ctx).await {
                    error!(
                        "Unable to delete own pin message {} in channel {}: {e:#?}",
                        new_message.id, new_message.channel_id
                    );
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Prepends each line with `> ` and ensures there's two newlines at the end, as text on the next line would become part of the quoteblock.
pub fn to_md_quote_block(comment: impl AsRef<str>) -> String {
    let comment = comment.as_ref();
    let mut out = String::with_capacity(comment.len() + 16);

    out += "> ";
    for char in comment.chars() {
        if char != '\n' {
            out.push(char);
        } else {
            out.push_str("\n> ");
        }
    }
    out.push_str("\n\n");

    out
}
