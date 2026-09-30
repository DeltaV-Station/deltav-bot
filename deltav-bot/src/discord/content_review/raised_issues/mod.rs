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
    subcommands("cr_issue_overview")
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

    match discussion
        .get_overrides_by_author(&ctx.data().db, message.author.id)
        .await
    {
        Ok(contention_messages) => {
            if contention_messages.contains(&message.id) {
                ctx.reply("Your contention can't also be an issue.").await?;
                return Ok(());
            }
        }
        Err(e) => {
            ctx.reply(e.to_string()).await?;
            return Ok(());
        }
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

    ctx.reply("Issue raised successfully.").await?;

    Ok(())
}

#[poise::command(context_menu_command = "Vote to override issues", ephemeral)]
pub async fn cr_issue_override_context(ctx: Context<'_>, message: Message) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    if ctx.author().id != message.author.id {
        ctx.reply("You can't contest issues using someone else's message.")
            .await?;
        return Ok(());
    }

    let discussion = match DiscussionRecord::get_by_thread(&ctx.data().db, message.channel_id).await
    {
        Some(x) => x,
        None => {
            ctx.reply("You can't contest an issue outside of a review thread.")
                .await?;
            return Ok(());
        }
    };

    match discussion
        .get_issues_by_author(&ctx.data().db, message.author.id)
        .await
    {
        Ok(issue_messages) => {
            if issue_messages.contains(&message.id) {
                ctx.reply("Your issue can't also be a contention.").await?;
                return Ok(());
            }
        }
        Err(e) => {
            error!(
                "Failed to get PR#{} issue for {} while trying to check against id of new override: {e}",
                message.author.id, discussion.pr_id
            );

            ctx.reply(e.to_string()).await?;
            return Ok(());
        }
    }

    if let Err(e) = discussion
        .upsert_issue_override(&ctx.data().db, message.author.id, message.id, None)
        .await
    {
        ctx.reply(format!("Failed to add contention: {e}")).await?;
    }

    if let Err(e) = message.pin(&ctx).await {
        error!(
            "Failed to pin message {} in {}: {e:#?}",
            message.id, message.channel_id
        );
        return Ok(());
    }

    ctx.reply("Contention recorded successfully.").await?;
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

    let issue_messages = match discussion
        .get_issues_by_author(&ctx.data().db, message.author.id)
        .await
    {
        Ok(x) => {
            if x.is_empty() {
                ctx.reply(format!("<@{}> has no active issue.", message.author.id))
                    .await?;
                return Ok(());
            }
            x
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

    let mut reply = CreateReply::default();

    for message in issue_messages {
        reply =
            reply.embed(create_message_embed(&ctx, &guild_channel, message, Some("issue")).await?);
    }

    ctx.send(reply).await?;

    Ok(())
}

#[poise::command(
    context_menu_command = "Dismiss selected issue or contention",
    ephemeral
)]
pub async fn cr_issue_dismiss_context(ctx: Context<'_>, message: Message) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    let Some(discussion) = DiscussionRecord::get_by_thread(&ctx.data().db, ctx.channel_id()).await
    else {
        ctx.reply("There is no PR associated with this channel.")
            .await?;
        return Ok(());
    };

    let author = match discussion
        .get_issue_author(&ctx.data().db, message.id)
        .await
    {
        Ok(author) => author,
        Err(e) => {
            ctx.reply(format!("Failed to retrieve issue author: {e}"))
                .await?;
            return Ok(());
        }
    };

    let mut is_author = false;
    let mut exists = false;
    let mut is_issue = false; // if false, override
    match author {
        Some(x) => {
            is_author = x == ctx.author().id;
            exists = true;
            is_issue = true;
        }
        None => {
            match discussion
                .get_override_author(&ctx.data().db, message.id)
                .await
            {
                Ok(Some(x)) => {
                    is_author = x == ctx.author().id;
                    exists = true;
                }

                Ok(None) => (),

                Err(e) => {
                    ctx.reply(format!("Failed to retrieve contention author: {e}"))
                        .await?;
                    return Ok(());
                }
            };
        }
    }

    if !exists {
        ctx.reply("The selected message has not been is not an issue or contention.")
            .await?;
        return Ok(());
    }

    if !is_author {
        ctx.reply("You can't dismiss someone else's issue or contention.")
            .await?;
        return Ok(());
    }

    if is_issue {
        if let Err(e) = discussion.delete_issue(&ctx.data().db, message.id).await {
            ctx.reply(format!("Failed to dismiss issue: {e}")).await?;
            return Ok(());
        }
    } else {
        if let Err(e) = discussion.delete_override(&ctx.data().db, message.id).await {
            ctx.reply(format!("Failed to dismiss contention: {e}"))
                .await?;
            return Ok(());
        }
    }

    message.unpin(&ctx).await?;

    ctx.reply(format!("Dismissed successfully.")).await?;
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

    let issues = discussion.get_issues(&db).await?;
    let overrides = discussion.get_overrides(&db).await?;

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
