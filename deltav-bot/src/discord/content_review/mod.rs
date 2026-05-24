use octocrab::params::pulls;
use poise::{
    ChoiceParameter,
    serenity_prelude::{
        ChannelId, CreateEmbed, CreateEmbedFooter, EditThread, ForumTagId, GuildChannel, RoleId,
    },
};
use tracing::error;

use crate::{
    discord::{
        Context, EMBED_DESC_MAX_LEN, Error,
        content_review::data::{
            config::Config,
            discussions::DiscussionRecord,
            forums::{ForumRecord, delete_forum_by_channel},
        },
        permissions::{check_permissions_command, data::PermissionFlags},
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

#[derive(ChoiceParameter)]
pub enum CrOutcome {
    #[name = "Test-Merge"]
    TestMerge,
    #[name = "Approved"]
    Approved,
    #[name = "Denied"]
    Denied,
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

#[poise::command(slash_command, subcommands("cr_forum", "cr_config", "cr_complete"))]
pub async fn cr(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

#[poise::command(slash_command, rename = "complete", ephemeral)]
pub async fn cr_complete(
    ctx: Context<'_>,
    outcome: CrOutcome,
    comment: Option<String>,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    let Some(mut channel) = ctx.guild_channel().await else {
        ctx.reply("Must be in a guild channel.").await?;
        return Ok(());
    };

    let Some(parent_channel) = channel.parent_id else {
        ctx.reply("Must be in a forum channel.").await?;
        return Ok(());
    };

    let Some(forum) = ForumRecord::get_by_channel(&ctx.data().db, parent_channel).await else {
        ctx.reply("Must be in a registered CR forum.").await?;
        return Ok(());
    };

    let Some(discussion) = DiscussionRecord::get_by_thread(&ctx.data().db, ctx.channel_id()).await
    else {
        ctx.reply("There is no PR associated with this thread.")
            .await?;
        return Ok(());
    };

    let Some(under_review_label) = Config::get_under_review_label(&ctx.data().db).await else {
        ctx.reply("Can't close with unset Under Review label.")
            .await?;
        return Ok(());
    };

    let gh = &ctx.data().gh;
    match outcome {
        CrOutcome::Approved | CrOutcome::TestMerge => {
            let Some(approved_label) = Config::get_approved_label(&ctx.data().db).await else {
                ctx.reply("Can't close with unset CR Approved label.")
                    .await?;
                return Ok(());
            };

            ctx.defer().await?;

            if let Err(e) = gh
                .octo_install
                .issues(&gh.repo_owner, &gh.repo_name)
                .add_labels(discussion.pr_id, &[approved_label])
                .await
            {
                error!(
                    "Failed to set CR Approved label on PR #{}: {e}",
                    discussion.pr_id
                );

                ctx.reply("Failed to add CR Approved GitHub label.").await?;
                return Ok(());
            };
        }
        CrOutcome::Denied => {
            let Some(denied_label) = Config::get_denied_label(&ctx.data().db).await else {
                ctx.reply("Can't close with unset CR Denied label.").await?;
                return Ok(());
            };

            ctx.defer().await?;

            if let Err(e) = gh
                .octo_install
                .issues(&gh.repo_owner, &gh.repo_name)
                .add_labels(discussion.pr_id, &[denied_label])
                .await
            {
                error!(
                    "Failed to set CR Denied label on PR #{}: {e}",
                    discussion.pr_id
                );

                ctx.reply("Failed to add CR Denied GitHub label.").await?;
                return Ok(());
            };

            if let Err(e) = gh
                .octo_install
                .pulls(&gh.repo_owner, &gh.repo_name)
                .update(discussion.pr_id)
                .state(pulls::State::Closed)
                .send()
                .await
            {
                error!("Failed to close PR #{}: {e}", discussion.pr_id);

                ctx.reply("Failed to close PR.").await?;
                // Not returning here since closing is really not essential, it's already marked
            }
        }
    }

    if let Err(e) = gh
        .octo_install
        .issues(&gh.repo_owner, &gh.repo_name)
        .remove_label(discussion.pr_id, &under_review_label)
        .await
    {
        error!(
            "Failed to remove Under Review label from PR #{}: {e}",
            discussion.pr_id
        );

        ctx.reply("Failed to remove Under Review GitHub label.")
            .await?;
        return Ok(());
    };

    if let Err(e) = gh
        .octo_install
        .issues(&gh.repo_owner, &gh.repo_name)
        .create_comment(
            discussion.pr_id,
            format!(
                "**CR consensus: {}**\n{}\n*Review closed by {}*",
                outcome.name(),
                comment.unwrap_or("No comment.".into()),
                ctx.author().name
            ),
        )
        .await
    {
        error!(
            "Failed to create CR outcome comment on PR #{}: {e}",
            discussion.pr_id
        );

        ctx.reply("Failed to create GitHub comment.").await?;
        return Ok(());
    };

    ctx.reply(format!(
        "This discussion has been closed: **{}**.",
        outcome.name()
    ))
    .await?;

    let tag = match outcome {
        CrOutcome::TestMerge => forum.tag_cr_test_merge,
        CrOutcome::Approved => forum.tag_cr_approved,
        CrOutcome::Denied => forum.tag_cr_denied,
    };

    if let Err(e) = channel
        .edit_thread(
            &ctx,
            EditThread::new()
                .archived(true)
                .applied_tags(channel.applied_tags.iter().chain([tag].iter()).cloned()),
        )
        .await
    {
        error!(
            "Failed to label and archive thread for PR#{} ({}): {e}",
            discussion.pr_id, channel.id
        );
        ctx.reply("Failed to label and archive thread. Lacking permissions.")
            .await?;
    }

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
#[poise::command(slash_command, rename = "config", ephemeral)]
pub async fn cr_config(
    ctx: Context<'_>,
    intake_cr_forum: Option<ChannelId>,
    public_cr_forum: Option<ChannelId>,
    private_cr_forum: Option<ChannelId>,
    gh_label_approved: Option<String>,
    gh_label_denied: Option<String>,
    gh_label_no_review: Option<String>,
    gh_label_under_review: Option<String>,
    review_ping_role: Option<RoleId>,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEW_CONFIG).await? {
        return Ok(());
    }

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

    if let Some(approved_label) = gh_label_approved {
        if let Err(e) = Config::set_approved_label(&ctx.data().db, approved_label).await {
            ctx.reply(format!("Error: {e}")).await?;
            return Ok(());
        }
    }

    if let Some(denied_label) = gh_label_denied {
        if let Err(e) = Config::set_denied_label(&ctx.data().db, denied_label).await {
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
#[poise::command(slash_command, rename = "upsert", ephemeral)]
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

// Delete a direction forum record (this does not delete the actual channel)
#[poise::command(slash_command, rename = "delete", ephemeral)]
pub async fn cr_forum_delete(ctx: Context<'_>, forum: ChannelId) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEW_CONFIG).await? {
        return Ok(());
    }

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
