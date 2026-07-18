use poise::{
    ChoiceParameter,
    serenity_prelude::{ChannelId, ForumTagId, RoleId},
};

use crate::discord::{
    Context, Error,
    content_review::data::{
        config::ignored::IgnoredKind,
        forums::{ForumRecord, delete_forum_by_channel},
    },
    permissions::{check_permissions_command, data::PermissionFlags},
};

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
#[poise::command(slash_command, rename = "config", ephemeral)]
pub async fn cr_config(
    ctx: Context<'_>,
    #[description = "The intake forum channel, where PRs are triaged"] intake_cr_forum: Option<
        ChannelId,
    >,
    #[description = "The public PR review forum channel"] public_cr_forum: Option<ChannelId>,
    #[description = "The private PR review forum channel"] private_cr_forum: Option<ChannelId>,
    #[description = "The full name of the GitHub label applied to approved PRs."] gh_label_approved: Option<String>,
    #[description = "The full name of the GitHub label applied to denied PRs."]
    gh_label_denied: Option<String>,
    #[description = "The full name of the GitHub label applied to PRs that don't need a review."]
    gh_label_no_review: Option<String>,
    #[description = "The full name of the GitHub label applied to PRs that are under review."]
    gh_label_under_review: Option<String>,
    #[description = "The full name of the GitHub label applied to PRs that require changes."]
    gh_label_changes_requested: Option<String>,
    #[description = "The content reviewer role, will get pinged for new reviews and review reminders."]
    review_ping_role: Option<RoleId>,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEW_CONFIG).await? {
        return Ok(());
    }

    let config = &ctx.data().cr_config;

    if let Some(intake_cr_forum) = intake_cr_forum {
        if let Err(e) = config.set_intake_forum(Some(intake_cr_forum)).await {
            ctx.reply(format!("Failed to set intake forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(public_cr_forum) = public_cr_forum {
        if let Err(e) = config.set_public_forum(Some(public_cr_forum)).await {
            ctx.reply(format!("Failed to set public forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(private_cr_forum) = private_cr_forum {
        if let Err(e) = config.set_private_forum(Some(private_cr_forum)).await {
            ctx.reply(format!("Failed to set private forum: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(no_review_needed_label) = gh_label_no_review {
        if let Err(e) = config
            .set_no_review_needed_label(no_review_needed_label)
            .await
        {
            ctx.reply(format!("Failed to set no review needed label: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(under_review_label) = gh_label_under_review {
        if let Err(e) = config.set_under_review_label(under_review_label).await {
            ctx.reply(format!("Failed to set under review label: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(approved_label) = gh_label_approved {
        if let Err(e) = config.set_approved_label(approved_label).await {
            ctx.reply(format!("Failed to set approved label: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(denied_label) = gh_label_denied {
        if let Err(e) = config.set_denied_label(denied_label).await {
            ctx.reply(format!("Failed to set denied label: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(review_ping_role) = review_ping_role {
        if let Err(e) = config.set_review_ping_role(Some(review_ping_role)).await {
            ctx.reply(format!("Failed to set review ping role: {e}"))
                .await?;
            return Ok(());
        }
    }

    if let Some(gh_label_changes_requested) = gh_label_changes_requested {
        if let Err(e) = config
            .set_changes_requested_label(gh_label_changes_requested)
            .await
        {
            ctx.reply(format!("Failed to set changes requested label: {e}"))
                .await?;
            return Ok(());
        }
    }

    ctx.reply("Processed without errors.").await?;
    Ok(())
}

/// Add or update a direction forum
#[poise::command(slash_command, rename = "upsert", ephemeral)]
pub async fn cr_forum_upsert(
    ctx: Context<'_>,
    #[description = "The forum channel"] forum: ChannelId,
    #[description = "Whether the forum is private"] private: bool,
    #[description = "The ID of the forum tag for approved PRs"] tag_approved: ForumTagId,
    #[description = "The ID of the forum tag for denied PRs"] tag_denied: ForumTagId,
    #[description = "The ID of the forum tag for PRs approved for a test-merge"]
    tag_test_merge: ForumTagId,
    #[description = "The ID of the forum tag for PRs that have been closed on GitHub"]
    tag_closed: ForumTagId,
    #[description = "The ID of the forum tag for PRs that have been merged"] tag_merged: ForumTagId,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEW_CONFIG).await? {
        return Ok(());
    }

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

/// Delete a direction forum record (this does not delete the actual channel)
#[poise::command(slash_command, rename = "delete", ephemeral)]
pub async fn cr_forum_delete(
    ctx: Context<'_>,
    #[description = "The forum channel"] forum: ChannelId,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEW_CONFIG).await? {
        return Ok(());
    }

    match delete_forum_by_channel(&ctx.data().db, &ctx.data().cr_config, forum).await {
        Ok(_) => {
            ctx.reply("Processed without errors.").await?;
        }
        Err(e) => {
            ctx.reply(format!("Failed to delete forum {e}.")).await?;
        }
    }

    Ok(())
}

#[poise::command(
    slash_command,
    rename = "ignored",
    subcommands("cr_ignored_list", "cr_ignored_add", "cr_ignored_remove")
)]
pub async fn cr_ignored(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

/// List all PR ignore criteria.
#[poise::command(slash_command, rename = "list", ephemeral)]
pub async fn cr_ignored_list(ctx: Context<'_>) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEW_CONFIG).await? {
        return Ok(());
    }

    let mut message = String::from("**PR Ignore Criteria**\n");

    let criteria = ctx.data().cr_config.ignored.get_all().await;
    for criterion in &criteria {
        message += &format!(
            "ID {}: {} `{}`\n",
            criterion.id,
            criterion.kind.name(),
            criterion.value
        );
    }

    if criteria.is_empty() {
        message += "None.";
    }

    ctx.reply(message).await?;
    Ok(())
}

/// Add a PR ignore criterion.
#[poise::command(slash_command, rename = "add", ephemeral)]
pub async fn cr_ignored_add(
    ctx: Context<'_>,
    #[description = "The kind of value"] kind: IgnoredKind,
    #[description = "PRs with matchign values will be ignored"] value: String,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEW_CONFIG).await? {
        return Ok(());
    }

    if let Err(e) = ctx.data().cr_config.ignored.add(kind, value).await {
        ctx.reply(format!("Failed to add ignore criterion: {e}"))
            .await?;
    }

    ctx.reply("Successfully added ignore criterion.").await?;
    Ok(())
}

/// Remove a specific PR ignore criterion. Use /cr ignored list to get the ID.
#[poise::command(slash_command, rename = "remove", ephemeral)]
pub async fn cr_ignored_remove(
    ctx: Context<'_>,
    #[description = "The ID from /cr ignored list"] id: String,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEW_CONFIG).await? {
        return Ok(());
    }

    let Ok(id) = id.parse::<i64>() else {
        ctx.reply("Specified ID is invalid.").await?;
        return Ok(());
    };

    if let Err(e) = ctx.data().cr_config.ignored.remove(id).await {
        ctx.reply(format!("Failed to remove ignore criterion: {e}"))
            .await?;
    }

    ctx.reply("Successfully removed ignore criterion.").await?;
    Ok(())
}
