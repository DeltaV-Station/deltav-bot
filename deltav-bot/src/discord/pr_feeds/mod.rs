use poise::serenity_prelude::{ChannelId, Mentionable, RoleId};
use tracing::{error, info};

use crate::discord::{
    Context, Error,
    permissions::{check_permissions_command, data::PermissionFlags},
};

pub mod data;

#[poise::command(
    slash_command,
    ephemeral,
    rename = "pr-feeds",
    subcommands("pr_feeds_add", "pr_feeds_list", "pr_feeds_remove")
)]
pub async fn pr_feeds(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(slash_command, ephemeral, rename = "add")]
pub async fn pr_feeds_add(
    ctx: Context<'_>,
    channel: ChannelId,
    github_label: String,
    ping_role: Option<RoleId>,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::PR_FEEDS_EDIT).await? {
        return Ok(());
    }

    let issues = ctx.data().gh.octo_install.issues_by_id(ctx.data().gh.repo);

    let labels = match issues.list_labels_for_repo().per_page(100).send().await {
        Ok(x) => x,
        Err(e) => {
            error!("Failed to fetch repo labels: {e:#?}");
            ctx.reply("Failed to fetch repo labels.").await?;
            return Ok(());
        }
    };

    if labels
        .items
        .iter()
        .find(|x| x.name == github_label)
        .is_none()
    {
        ctx.reply("The specified label does not exist.").await?;
        return Ok(());
    }

    if let Err(e) = ctx
        .data()
        .pr_feeds
        .add(&github_label, channel, ping_role)
        .await
    {
        ctx.reply(e.to_string()).await?;
        return Ok(());
    }

    info!(
        "{} added PR feed: label {github_label}; channel {channel}; ping role {ping_role:?}",
        ctx.author().name
    );

    ctx.reply("Successfully added feed.").await?;
    Ok(())
}

#[poise::command(slash_command, ephemeral, rename = "list")]
pub async fn pr_feeds_list(ctx: Context<'_>) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::PR_FEEDS_EDIT).await? {
        return Ok(());
    }

    let mut message = String::from("**Current PR Feeds**\n");

    let feeds = ctx.data().pr_feeds.get_all().await;
    for feed in &feeds {
        message += &format!(
            "ID {}: Label '{}' to <#{}>, pinging {}\n",
            feed.id,
            feed.gh_label,
            feed.channel_id,
            feed.ping_role
                .and_then(|x| Some(x.mention().to_string()))
                .unwrap_or("nobody".into())
        );
    }

    if feeds.len() == 0 {
        message += "None.";
    }

    ctx.reply(message).await?;
    Ok(())
}

#[poise::command(slash_command, ephemeral, rename = "remove")]
pub async fn pr_feeds_remove(ctx: Context<'_>, feed_id: String) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::PR_FEEDS_EDIT).await? {
        return Ok(());
    }

    let Ok(feed_id) = feed_id.parse::<i64>() else {
        ctx.reply("Invalid feed ID. Must be a number.").await?;
        return Ok(());
    };

    if let Err(e) = ctx.data().pr_feeds.remove(feed_id).await {
        ctx.reply(e.to_string()).await?;
        return Ok(());
    }

    info!("{} deleted PR feed {feed_id}", ctx.author().name);

    ctx.reply("Successfully removed feed.").await?;
    Ok(())
}
