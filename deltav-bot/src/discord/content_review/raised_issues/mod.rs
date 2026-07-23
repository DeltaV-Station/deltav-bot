use poise::{
    CreateReply,
    serenity_prelude::{
        Cache, CacheHttp, CreateEmbed, EMBED_MAX_LENGTH, GuildChannel, Mentionable, Message,
        MessageId,
    },
};
use sqlx::{Pool, Sqlite};
use tracing::error;

use crate::discord::{
    Context, Error, HandledError,
    content_review::data::discussions::DiscussionRecord,
    permissions::{check_permissions_command, data::PermissionFlags},
};

pub mod comp_tasks;

#[poise::command(
    slash_command,
    rename = "issue",
    ephemeral,
    subcommands("cr_issue_dismiss", "cr_issue_dismiss_override", "cr_issue_overview")
)]
pub async fn cr_issue(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

#[poise::command(slash_command, rename = "overview", ephemeral)]
/// List all issues and overrides.
pub async fn cr_issue_overview(ctx: Context<'_>) -> Result<(), Error> {
    cr_issue_overview_impl(&ctx).await?;
    Ok(())
}

#[poise::command(context_menu_command = "Issue overview", ephemeral)]
pub async fn cr_issue_overview_context(ctx: Context<'_>, _message: Message) -> Result<(), Error> {
    cr_issue_overview_impl(&ctx).await?;
    Ok(())
}

async fn cr_issue_overview_impl(ctx: &Context<'_>) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    let discussion = match DiscussionRecord::get_by_thread(&ctx.data().db, ctx.channel_id()).await {
        Some(x) => x,
        None => {
            ctx.reply(
                "Issues can only be raised in review threads, there is nothing to view here.",
            )
            .await?;
            return Ok(());
        }
    };

    let mut embeds = match create_issue_overview_embeds(&ctx, &ctx.data().db, &discussion).await {
        Ok(x) => x,
        Err(e) => {
            ctx.reply(format!("Failed to create overview: {e}")).await?;
            return Ok(());
        }
    };

    let mut embeds = embeds.drain(..);
    let mut message = CreateReply::default();
    let mut message_embeds = 0;

    while let Some(embed) = embeds.next() {
        if message_embeds == 10 {
            let _ = ctx.send(message).await;
            message = CreateReply::default();
            message_embeds = 0;
        }

        message = message.embed(embed);
        message_embeds += 1;
    }

    if message_embeds != 0 {
        let _ = ctx.send(message).await;
    }

    Ok(())
}

#[poise::command(context_menu_command = "Raise issue", ephemeral)]
pub async fn cr_issue_raise_context(ctx: Context<'_>, message: Message) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    if ctx.author().id != message.author.id {
        ctx.reply("You can't mark someone else's message as your raised issue.")
            .await?;
        return Ok(());
    }

    let discussion = match DiscussionRecord::get_by_thread(&ctx.data().db, message.channel_id).await
    {
        Some(x) => x,
        None => {
            ctx.reply("You can't raise an issue outside of a review thread.")
                .await?;
            return Ok(());
        }
    };

    let old_message = match discussion
        .get_issue_by_author(&ctx.data().db, ctx.author().id)
        .await
    {
        Ok(x) => x,
        Err(e) => {
            ctx.reply(format!("Failed to check for previous issue: {e}"))
                .await?;
            return Ok(());
        }
    };

    match discussion
        .get_override_by_author(&ctx.data().db, ctx.author().id)
        .await
    {
        Ok(Some(x)) => {
            if message.id == x {
                ctx.reply("Your override can't also be an issue.").await?;
                return Ok(());
            }
        }
        Ok(None) => (),
        Err(e) => {
            ctx.reply(format!("Failed to check for override: {e}"))
                .await?;
            return Ok(());
        }
    };

    if old_message
        .and_then(|x| Some(x == message.id))
        .unwrap_or_default()
    {
        ctx.reply("You've already marked this message as your raised issue.")
            .await?;
        return Ok(());
    }

    if let Err(e) = discussion
        .upsert_issue(&ctx.data().db, ctx.author().id, message.id)
        .await
    {
        ctx.reply(format!("Failed to raise issue: {e}")).await?;
        return Ok(());
    }

    if let Err(e) = message.pin(&ctx).await {
        error!(
            "Failed to pin message {} in {}: {e:#?}",
            message.id, message.channel_id
        );

        ctx.reply("Failed to pin message. Lacking permission?")
            .await?;
        return Ok(());
    }

    let Some(channel) = ctx.guild_channel().await else {
        error!("Channel for {discussion:?} wasn't a guild channel.");
        return Ok(());
    };

    if let Some(old_message) = old_message {
        match channel.message(&ctx, old_message).await {
            Ok(x) => {
                x.unpin(&ctx).await?;
            }
            Err(e) => {
                error!(
                    "Failed to resolve {}'s old issue message {old_message}: {e:#?}",
                    ctx.author().id
                );

                // Might've already been deleted, not going to bug the user about it or abort
            }
        }
    }

    let overrides = match discussion.clear_issue_overrides(&ctx.data().db).await {
        Ok(x) => x,
        Err(e) => {
            ctx.reply(format!("Failed to retrieve overrides: {e}"))
                .await?;
            return Ok(());
        }
    };

    for (_, message_id) in overrides {
        match channel.message(&ctx, message_id).await {
            Ok(x) => {
                x.unpin(&ctx).await?;
            }
            Err(e) => {
                error!(
                    "Failed to resolve old override message {message_id} in {discussion:?}: {e:#?}"
                );
                // Might've already been deleted, not going to bug the user about it or abort
            }
        }
    }

    ctx.reply("Issue raised successfully.").await?;

    Ok(())
}

#[poise::command(context_menu_command = "Vote to override issues", ephemeral)]
pub async fn cr_issue_override_context(ctx: Context<'_>, message: Message) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    if ctx.author().id != message.author.id {
        ctx.reply("You can't mark someone else's message as your issue override vote.")
            .await?;
        return Ok(());
    }

    let discussion = match DiscussionRecord::get_by_thread(&ctx.data().db, message.channel_id).await
    {
        Some(x) => x,
        None => {
            ctx.reply("You can't override an issue outside of a review thread.")
                .await?;
            return Ok(());
        }
    };

    let Some(guild_channel) = ctx.guild_channel().await else {
        error!("Channel for {discussion:?} wasn't a guild channel.");
        return Ok(());
    };

    match discussion
        .get_issue_by_author(&ctx.data().db, message.author.id)
        .await
    {
        Ok(Some(issue_message)) => {
            if message.id == issue_message {
                ctx.reply("You can't mark your issue as one of its overrides.")
                    .await?;
                return Ok(());
            }
        }
        Ok(None) => (),
        Err(e) => {
            error!(
                "Failed to get PR#{} issue for {} while trying to check against id of new override: {e}",
                message.author.id, discussion.pr_id
            );
        }
    }

    let old_message = match discussion
        .get_override_by_author(&ctx.data().db, message.author.id)
        .await
    {
        Ok(x) => x,
        Err(e) => {
            ctx.reply(format!("Failed to check for previous override: {e}"))
                .await?;
            return Ok(());
        }
    };

    if old_message
        .and_then(|x| Some(x == message.id))
        .unwrap_or_default()
    {
        ctx.reply(
            "You've already marked this message as your override for <@{issue_author}>'s issue.",
        )
        .await?;
        return Ok(());
    }

    if let Err(e) = discussion
        .upsert_issue_override(&ctx.data().db, message.author.id, message.id)
        .await
    {
        ctx.reply(format!("Failed to add issue override: {e}"))
            .await?;
    }

    if let Err(e) = message.pin(&ctx).await {
        error!(
            "Failed to pin message {} in {}: {e:#?}",
            message.id, message.channel_id
        );
        return Ok(());
    }

    if let Some(old_message) = old_message {
        match guild_channel.message(&ctx, old_message).await {
            Ok(x) => {
                x.unpin(&ctx).await?;
            }
            Err(e) => {
                error!(
                    "Failed to resolve {}'s old issue override message {old_message}: {e:#?}",
                    ctx.author().id
                );

                ctx.reply("Failed to resolve old issue override message, assuming it was deleted. The new issue override has been successfully recorded and pinned, but no attempt to unpin the old issue will be made.").await?;
            }
        }
    }

    ctx.reply("Override vote added successfully.").await?;

    Ok(())
}

#[poise::command(context_menu_command = "View author's issue", ephemeral)]
pub async fn cr_issue_view_context(ctx: Context<'_>, message: Message) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    let discussion = match DiscussionRecord::get_by_thread(&ctx.data().db, message.channel_id).await
    {
        Some(x) => x,
        None => {
            ctx.reply(
                "Issues can only be raised in review threads, there are no overrides to view here.",
            )
            .await?;
            return Ok(());
        }
    };

    let issue_message = match discussion
        .get_issue_by_author(&ctx.data().db, message.author.id)
        .await
    {
        Ok(Some(x)) => x,
        Ok(None) => {
            ctx.reply(format!("<@{}> has no active issue.", message.author.id))
                .await?;
            return Ok(());
        }
        Err(e) => {
            ctx.reply(format!(
                "Failed to check for issue associated with author: {e}"
            ))
            .await?;
            return Ok(());
        }
    };

    let Some(guild_channel) = ctx.guild_channel().await else {
        error!("Channel for {discussion:?} wasn't a guild channel.");
        return Ok(());
    };

    ctx.send(
        CreateReply::default()
            .embed(create_message_embed(&ctx, &guild_channel, issue_message, Some("issue")).await?),
    )
    .await?;

    Ok(())
}

#[poise::command(context_menu_command = "Dismiss own issue", ephemeral)]
pub async fn cr_issue_dismiss_context(ctx: Context<'_>, _message: Message) -> Result<(), Error> {
    dismiss_own_impl(ctx, false).await?;
    Ok(())
}

#[poise::command(slash_command, rename = "dismiss", ephemeral)]
/// Dismiss the issue you raised.
pub async fn cr_issue_dismiss(ctx: Context<'_>) -> Result<(), Error> {
    dismiss_own_impl(ctx, false).await?;
    Ok(())
}

#[poise::command(context_menu_command = "Dismiss own override", ephemeral)]
pub async fn cr_issue_dismiss_override_context(
    ctx: Context<'_>,
    _message: Message,
) -> Result<(), Error> {
    dismiss_own_impl(ctx, true).await?;
    Ok(())
}

#[poise::command(slash_command, rename = "dismiss-override", ephemeral)]
/// Dismiss your vote to override.
pub async fn cr_issue_dismiss_override(ctx: Context<'_>) -> Result<(), Error> {
    dismiss_own_impl(ctx, true).await?;
    Ok(())
}

/// if !is_override, dismiss issue. if is_override, dismiss override
async fn dismiss_own_impl(ctx: Context<'_>, is_override: bool) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    let Some(discussion) = DiscussionRecord::get_by_thread(&ctx.data().db, ctx.channel_id()).await
    else {
        ctx.reply("There is no PR associated with this channel.")
            .await?;
        return Ok(());
    };

    let message = if is_override {
        discussion
            .delete_override_by_author(&ctx.data().db, ctx.author().id)
            .await?
    } else {
        discussion
            .delete_issue_by_author(&ctx.data().db, ctx.author().id)
            .await?
    };

    match message {
        Some(x) => {
            let channel = ctx
                .guild_channel()
                .await
                .ok_or(HandledError::UserfacingError(
                    "Issue/override dismissed outside of guild.".into(),
                ))?;

            channel.message(&ctx, x).await?.unpin(&ctx).await?
        }
        None => {
            ctx.reply(format!(
                "You haven't {} in this discussion.",
                if is_override {
                    "voted to override the issues"
                } else {
                    "raised an issue"
                }
            ))
            .await?;

            return Ok(());
        }
    }

    ctx.reply(format!(
        "{} dismissed successfully.",
        if is_override { "Override" } else { "Issue" }
    ))
    .await?;
    Ok(())
}

pub async fn create_message_embed(
    ctx: impl CacheHttp + AsRef<Cache>,
    channel: &GuildChannel,
    message_id: MessageId,
    message_label_override: Option<impl Into<String>>,
) -> Result<CreateEmbed, Error> {
    let message_label = message_label_override
        .and_then(|x| Some(x.into()))
        .unwrap_or("message".into());

    let message = match channel.message(&ctx, message_id).await {
        Ok(x) => x,
        Err(e) => {
            error!(
                "Failed to retrieve message while creating embed for issue with message ID {message_id} in {channel}: {e:#?}"
            );

            return Ok(CreateEmbed::new()
                .title(format!("Unknown {message_label}"))
                .description("Failed to retrieve message.")
                .url(message_id.link(channel.id, Some(channel.guild_id))));
        }
    };

    let author_name = &message.author.name;
    let message_content_truncated = message
        .content_safe(&ctx)
        .chars()
        .take(EMBED_MAX_LENGTH)
        .collect::<String>();

    Ok(CreateEmbed::new()
        .title(format!("{author_name}'s {message_label}",))
        .url(message_id.link(channel.id, Some(channel.guild_id)))
        .description(message_content_truncated)
        .field("Author", message.author.mention().to_string(), true))
}

pub async fn create_issue_overview_embeds(
    ctx: impl CacheHttp + AsRef<Cache>,
    db: &Pool<Sqlite>,
    discussion: &DiscussionRecord,
) -> Result<Vec<CreateEmbed>, HandledError> {
    let discussion_channel = discussion
        .thread_id
        .to_channel(&ctx)
        .await
        .map_err(|e| {
            error!("Failed to get channel {}: {e}", discussion.thread_id);

            HandledError::InternalError
        })?
        .guild()
        .ok_or(HandledError::InternalError)?;

    let issues = discussion.get_raised_issues(&db).await?;
    let overrides = discussion.get_issue_overrides(&db).await?;

    let mut embeds = vec![];

    for (user, message) in issues {
        let embed =
            match create_message_embed(&ctx, &discussion_channel, message, Some("issue")).await {
                Ok(x) => x,
                Err(e) => {
                    error!(
                        "Failed to create issue embed for {user}'s message {message} in {}: {e}",
                        discussion_channel.id
                    );
                    continue;
                }
            };

        embeds.push(embed);
    }

    for (user, message) in overrides {
        let embed = match create_message_embed(&ctx, &discussion_channel, message, Some("override"))
            .await
        {
            Ok(x) => x,
            Err(e) => {
                error!(
                    "Failed to create override embed for {user}'s message {message} in {}: {e}",
                    discussion_channel.id
                );
                continue;
            }
        };

        embeds.push(embed);
    }

    Ok(embeds)
}
