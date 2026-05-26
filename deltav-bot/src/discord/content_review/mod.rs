use octocrab::params::pulls;
use poise::{
    ChoiceParameter, CreateReply, Modal,
    serenity_prelude::{
        ChannelId, CreateEmbed, CreateEmbedFooter, EditThread, ForumTagId, GuildChannel, RoleId,
    },
};
use tracing::error;

use crate::{
    discord::{
        ApplicationContext, Context, EMBED_DESC_MAX_LEN, Error,
        content_review::data::{
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
pub mod timers;

pub const INTERACTION_ID_PREFIX: &'static str = "cr";
pub const BUTTON_ID_ACTION_START_PUBLIC: &'static str = "reviewStartPublic";
pub const BUTTON_ID_ACTION_START_PRIVATE: &'static str = "reviewStartPrivate";
pub const BUTTON_ID_ACTION_NOT_NEEDED: &'static str = "reviewNotNeeded";
pub const BUTTON_ID_ACTION_MUTE_REMINDERS: &'static str = "reviewRemindersStop";

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

#[derive(Debug, Modal)]
#[name = "Request changes"]
struct RequestChangesModal {
    #[name = "What should be changed?"]
    #[placeholder = "If you prefer writing the comment yourself, leave this field blank to only add the label."]
    #[paragraph]
    description: String,
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

#[poise::command(
    slash_command,
    subcommands("cr_forum", "cr_config", "cr_complete", "cr_request_changes")
)]
pub async fn cr(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

#[poise::command(slash_command, rename = "request-changes", ephemeral)]
pub async fn cr_request_changes(ctx: ApplicationContext<'_>) -> Result<(), Error> {
    let wrapped_ctx = Context::Application(ctx);
    if !check_permissions_command(&wrapped_ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    let (_, mut discussion, _) = match get_channel_discussion(&wrapped_ctx).await {
        Ok((forum, discussion, channel)) => (forum, discussion, channel),
        Err(e) => {
            ctx.reply(format!("Error: {e}")).await?;
            return Ok(());
        }
    };

    let Some(under_review_label) = ctx.data().cr_config.get_under_review_label().await else {
        ctx.reply("Can't process change request with under review label unset.")
            .await?;
        return Ok(());
    };

    let Some(changes_requested_label) = ctx.data().cr_config.get_changes_requested_label().await
    else {
        ctx.reply("Can't process change request with changes requested label unset.")
            .await?;
        return Ok(());
    };

    let Some(response) = RequestChangesModal::execute(ctx).await? else {
        ctx.reply("Modal timed out.").await?;
        return Ok(());
    };

    let issues = ctx.data().gh.octo_install.issues_by_id(ctx.data().gh.repo);

    if !response.description.is_empty() {
        if let Err(e) = issues
            .create_comment(
                discussion.pr_id,
                format!(
                    "**Changes requested by CR**\n```\n{}\n```\nSent by {}.",
                    response.description,
                    ctx.author().name
                ),
            )
            .await
        {
            error!(
                "Failed to create change request comment on PR #{}: {e:#?}",
                discussion.pr_id
            );

            ctx.reply("Failed to create GitHub comment").await?;
            return Ok(());
        }
    }

    ctx.send(CreateReply::default()
        .ephemeral(false)
        .embed(CreateEmbed::new()
            .title("Changes requested")
            .description(
                if response.description.is_empty() {
                    "Description was unset. Label has been applied, please write a comment describing the required changes yourself.".into()
                } else
                {
                    response.description
                })
            .footer(CreateEmbedFooter::new(&ctx.author().name))
        )
    ).await?;

    if let Err(e) = issues
        .add_labels(discussion.pr_id, &[changes_requested_label])
        .await
    {
        error!(
            "Failed to set changes requested label on PR #{}: {e:#?}",
            discussion.pr_id
        );

        ctx.reply("Failed to set changes requested label").await?;
        return Ok(());
    }

    if let Err(e) = issues
        .remove_label(discussion.pr_id, under_review_label)
        .await
    {
        let did_label_exist = if let octocrab::Error::GitHub {
            source,
            backtrace: _,
        } = &e
            && source.status_code == 410
        {
            false
        } else {
            true
        };

        if !did_label_exist {
            error!(
                "Failed to remove under review label from PR #{}: {e:#?}",
                discussion.pr_id
            );

            ctx.reply("Failed to remove under review label").await?;
            return Ok(());
        }
    }

    if let Err(e) = discussion.disable_reminders(&ctx.data().db).await {
        ctx.reply(format!(
            "Failed to disable reminders upon change request: {e}"
        ))
        .await?;
    }

    Ok(())
}

async fn get_channel_discussion(
    ctx: &Context<'_>,
) -> Result<(ForumRecord, DiscussionRecord, GuildChannel), HandledError> {
    let Some(channel) = ctx.guild_channel().await else {
        return Err(HandledError::UserfacingError(
            "Must be in a guild channel.".into(),
        ));
    };

    let Some(parent_channel) = channel.parent_id else {
        return Err(HandledError::UserfacingError(
            "Must be in a forum channel.".into(),
        ));
    };

    let Some(forum) = ForumRecord::get_by_channel(&ctx.data().db, parent_channel).await else {
        return Err(HandledError::UserfacingError(
            "Must be in a registered CR forum.".into(),
        ));
    };

    let Some(discussion) = DiscussionRecord::get_by_thread(&ctx.data().db, ctx.channel_id()).await
    else {
        return Err(HandledError::UserfacingError(
            "There is no PR associated with this thread.".into(),
        ));
    };

    Ok((forum, discussion, channel))
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

    let (forum, mut discussion, mut channel) = match get_channel_discussion(&ctx).await {
        Ok((forum, discussion, channel)) => (forum, discussion, channel),
        Err(e) => {
            ctx.reply(format!("Error: {e}")).await?;
            return Ok(());
        }
    };

    let Some(under_review_label) = ctx.data().cr_config.get_under_review_label().await else {
        ctx.reply("Can't close with unset Under Review label.")
            .await?;
        return Ok(());
    };

    let gh = &ctx.data().gh;
    let config = &ctx.data().cr_config;
    let issues = gh.octo_install.issues_by_id(gh.repo);
    match outcome {
        CrOutcome::Approved | CrOutcome::TestMerge => {
            let Some(approved_label) = config.get_approved_label().await else {
                ctx.reply("Can't close with unset CR Approved label.")
                    .await?;
                return Ok(());
            };

            ctx.defer().await?;

            if let Err(e) = issues.add_labels(discussion.pr_id, &[approved_label]).await {
                error!(
                    "Failed to set CR Approved label on PR #{}: {e}",
                    discussion.pr_id
                );

                ctx.reply("Failed to add CR Approved GitHub label.").await?;
                return Ok(());
            };
        }
        CrOutcome::Denied => {
            let Some(denied_label) = config.get_denied_label().await else {
                ctx.reply("Can't close with unset CR Denied label.").await?;
                return Ok(());
            };

            ctx.defer().await?;

            if let Err(e) = issues.add_labels(discussion.pr_id, &[denied_label]).await {
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

    if let Err(e) = discussion.disable_reminders(&ctx.data().db).await {
        ctx.reply(format!("Failed to disable reminders during closing: {e}"))
            .await?;
    }

    if let Err(e) = issues
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

    if let Err(e) = issues
        .create_comment(
            discussion.pr_id,
            format!(
                "**CR consensus: {}**\n```\n{}\n```\nReview closed by {}.",
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
    gh_label_changes_requested: Option<String>,
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
