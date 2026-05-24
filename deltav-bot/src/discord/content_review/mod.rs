use poise::serenity_prelude::{
    ChannelId, CreateEmbed, CreateEmbedFooter, ForumTagId, GuildChannel, RoleId,
};
use tracing::error;

use crate::{
    discord::{
        Context, EMBED_DESC_MAX_LEN, Error,
        content_review::data::{
            config::Config,
            forums::{ForumRecord, delete_forum_by_channel},
        },
    },
    github::GitHub,
};

pub mod component_events;
pub mod data;
pub mod github_events;

pub const INTERACTION_ID_PREFIX: &'static str = "cr";
pub const BUTTON_ID_ACTION_START_PUBLIC: &'static str = "reviewStartPublic";
pub const BUTTON_ID_ACTION_START_PRIVATE: &'static str = "reviewStartPrivate";
pub const BUTTON_ID_ACTION_NOT_NEEDED: &'static str = "reviewNotNeeded";

/// Error returned by underlying systems, denoting how the error should be presented to the user.
/// If a function returns this type of Error, it must properly log all errors using `tracing::error`.
#[derive(thiserror::Error, Debug)]
pub enum HandledError {
    #[error("{0}")]
    UserfacingError(String),
    #[error("An internal error occurred")]
    InternalError,
}

async fn discussion_channel_to_guild(
    pr_id: u64,
    id: ChannelId,
    ctx: &poise::serenity_prelude::Context,
) -> Option<GuildChannel> {
    let guild_channel = match id.to_channel(ctx).await {
        Ok(x) => x,
        Err(e) => {
            error!("Failed to fetch channel from id {id}: {e:#?}");
            return None;
        }
    };

    let guild_channel = guild_channel.guild();
    if guild_channel.is_none() {
        error!("Discussion channel for PR {pr_id} was not a guild channel!");
    };

    guild_channel
}

#[poise::command(slash_command, subcommands("cr_forum", "cr_config", "cr_review"))]
pub async fn cr(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

#[poise::command(slash_command, rename = "review")]
pub async fn cr_review(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

#[poise::command(
    slash_command,
    subcommands("cr_forum_upsert", "cr_forum_delete"),
    rename = "forum"
)]
pub async fn cr_forum(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

/// Set config values for the Content Review module
#[poise::command(slash_command, rename = "config")]
pub async fn cr_config(
    ctx: Context<'_>,
    intake_cr_forum: Option<ChannelId>,
    public_cr_forum: Option<ChannelId>,
    private_cr_forum: Option<ChannelId>,
    gh_label_no_review: Option<String>,
    gh_label_under_review: Option<String>,
    review_ping_role: Option<RoleId>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    if let Some(intake_cr_forum) = intake_cr_forum {
        if let Err(e) = Config::set_intake_forum(&ctx.data().db, Some(intake_cr_forum)).await {
            ctx.reply(format!("Failed to set intake forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(public_cr_forum) = public_cr_forum {
        if let Err(e) = Config::set_public_forum(&ctx.data().db, Some(public_cr_forum)).await {
            ctx.reply(format!("Failed to set public forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(private_cr_forum) = private_cr_forum {
        if let Err(e) = Config::set_private_forum(&ctx.data().db, Some(private_cr_forum)).await {
            ctx.reply(format!("Failed to set private forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(no_review_needed_label) = gh_label_no_review {
        if let Err(e) =
            Config::set_no_review_needed_label(&ctx.data().db, no_review_needed_label).await
        {
            ctx.reply(format!("Error: {e}")).await?;
            return Ok(());
        }
    }

    if let Some(under_review_label) = gh_label_under_review {
        if let Err(e) = Config::set_under_review_label(&ctx.data().db, under_review_label).await {
            ctx.reply(format!("Error: {e}")).await?;
            return Ok(());
        }
    }

    if let Some(review_ping_role) = review_ping_role {
        if let Err(e) = Config::set_review_ping_role(&ctx.data().db, Some(review_ping_role)).await {
            ctx.reply(format!("Error: {e}")).await?;
            return Ok(());
        }
    }

    ctx.reply("Processed without errors.").await?;
    Ok(())
}

/// Add or update a direction forum
#[poise::command(slash_command, rename = "upsert")]
pub async fn cr_forum_upsert(
    ctx: Context<'_>,
    forum: ChannelId,
    private: bool,
    tag_approved: ForumTagId,
    tag_denied: ForumTagId,
    tag_test_merge: ForumTagId,
    tag_closed: ForumTagId,
    tag_merged: ForumTagId,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    let record = ForumRecord {
        channel_id: forum,
        private,
        tag_cr_approved: tag_approved,
        tag_cr_denied: tag_denied,
        tag_cr_test_merge: tag_test_merge,
        tag_pr_closed: tag_closed,
        tag_pr_merged: tag_merged,
    };

    match record.upsert(&ctx.data().db).await {
        Ok(_) => {
            ctx.reply("Processed without errors.").await?;
        }
        Err(e) => {
            ctx.reply(format!("Failed to upsert forum: {e}")).await?;
        }
    }

    Ok(())
}

// Delete a direction forum record (this does not delete the actual channel)
#[poise::command(slash_command, rename = "delete")]
pub async fn cr_forum_delete(ctx: Context<'_>, forum: ChannelId) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    match delete_forum_by_channel(&ctx.data().db, forum).await {
        Ok(_) => {
            ctx.reply("Processed without errors.").await?;
        }
        Err(e) => {
            ctx.reply(format!("Failed to delete forum {e}.")).await?;
        }
    }

    Ok(())
}

pub fn create_pr_embed(
    pr_id: u64,
    pr_title: String,
    pr_author: String,
    pr_body: Option<String>,
    gh: &GitHub,
) -> CreateEmbed {
    // String::truncate might panic, so doing it like this.
    let embed_description: String = pr_body
        .unwrap_or("No description.".into())
        .chars()
        .take(EMBED_DESC_MAX_LEN)
        .collect();

    CreateEmbed::new()
        .footer(CreateEmbedFooter::new(format!(
            "PR #{pr_id}, submitted by {pr_author}"
        )))
        .url(format!(
            "https://github.com/{}/{}/pull/{pr_id}",
            gh.repo_owner, gh.repo_name
        ))
        .title(&pr_title)
        .description(&embed_description)
}
