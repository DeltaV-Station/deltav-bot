use std::time::Duration;

use poise::{
    CreateReply,
    serenity_prelude::{
        ComponentInteractionCollector, ComponentInteractionDataKind, CreateActionRow,
        CreateSelectMenu, CreateSelectMenuKind, Message,
    },
};
use tracing::error;

use crate::discord::{
    Context, Error,
    content_review::{INTERACTION_ID_PREFIX, data::discussions::DiscussionRecord},
    permissions::{check_permissions_command, data::PermissionFlags},
};

pub const SELECT_ID_ISSUE_OVERRIDE: &'static str = "overrideIssue";

#[poise::command(context_menu_command = "Raise issue", ephemeral)]
pub async fn cr_issue_raise(ctx: Context<'_>, message: Message) -> Result<(), Error> {
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

    let old_message = match discussion.get_issue(&ctx.data().db, ctx.author().id).await {
        Ok(x) => x,
        Err(e) => {
            ctx.reply(format!("Failed to check for previous issue: {e}"))
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
        return Ok(());
    }

    if let Some(old_message) = old_message {
        match ctx
            .guild_channel()
            .await
            .expect("We already know this is a guild channel")
            .message(&ctx, old_message)
            .await
        {
            Ok(x) => {
                x.unpin(&ctx).await?;
            }
            Err(e) => {
                error!(
                    "Failed to resolve {}'s old issue message {old_message}: {e:#?}",
                    ctx.author().id
                );

                ctx.reply("Failed to resolve old issue message, assuming it was deleted. The new issue has been successfully recorded and pinned, but no attempt to unpin the old issue will be made.").await?;
            }
        }
    }

    ctx.reply("Issue raised successfully.").await?;

    Ok(())
}

#[poise::command(context_menu_command = "Vote to override", ephemeral)]
pub async fn cr_issue_override(ctx: Context<'_>, message: Message) -> Result<(), Error> {
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

    let guild_channel = ctx
        .guild_channel()
        .await
        .expect("We already know this is a guild channel");

    // fetch full message as reply data isn't returned on context menu interaction
    let message = guild_channel.message(ctx, message).await?;

    let mut issue_author = None;
    if let Some(replied_to) = &message.referenced_message {
        match discussion
            .get_issue(&ctx.data().db, replied_to.author.id)
            .await
        {
            Ok(Some(issue_message)) => {
                if replied_to.id == issue_message {
                    issue_author = Some(replied_to.author.id);
                }
            }
            Ok(None) => (),
            Err(e) => {
                ctx.reply(format!(
                    "Failed to check issue for author of message that was replied to: {e}"
                ))
                .await?;
                return Ok(());
            }
        }
    }

    if issue_author.is_none() {
        let select_id = format!(
            "{INTERACTION_ID_PREFIX}_{SELECT_ID_ISSUE_OVERRIDE}_{}",
            discussion.pr_id
        );

        let response = ctx
            .send(
                CreateReply::default()
                    .content("Whose issue would you like to override?")
                    .components(vec![CreateActionRow::SelectMenu(CreateSelectMenu::new(
                        &select_id,
                        CreateSelectMenuKind::User {
                            default_users: None,
                        },
                    ))]),
            )
            .await?;

        let response_id = response.message().await?.id;
        let users = match ComponentInteractionCollector::new(&ctx)
            .message_id(response_id)
            .custom_ids(vec![select_id])
            .timeout(Duration::from_secs(120))
            .await
        {
            Some(x) => {
                x.create_response(
                    &ctx,
                    poise::serenity_prelude::CreateInteractionResponse::Acknowledge,
                )
                .await?;

                if let ComponentInteractionDataKind::UserSelect { values } = x.data.kind {
                    values
                } else {
                    ctx.reply("Invalid interaction received.").await?;
                    return Ok(());
                }
            }
            None => {
                ctx.reply("Interaction aborted due to inactivity.").await?;
                return Ok(());
            }
        };

        let _ = response.delete(ctx).await; // non-critical

        if users.len() != 1 {
            ctx.reply("You must select exactly 1 user.").await?;
            return Ok(());
        }

        issue_author = Some(users[0]);
    }

    let issue_author =
        issue_author.expect("If no valid user was provided, we should've already returned");

    match discussion.get_issue(&ctx.data().db, issue_author).await {
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
                "Failed to get PR#{} issue for {issue_author} while trying to check against id of new override: {e}",
                discussion.pr_id
            );
        }
    }

    let old_message = match discussion
        .get_issue_override(&ctx.data().db, issue_author, ctx.author().id)
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
        .upsert_issue_override(&ctx.data().db, issue_author, message.author.id, message.id)
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
pub async fn cr_issue_view(ctx: Context<'_>, message: Message) -> Result<(), Error> {
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
        .get_issue(&ctx.data().db, message.author.id)
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

    let overrides = match discussion
        .get_issue_overrides(&ctx.data().db, message.author.id)
        .await
    {
        Ok(x) => x,
        Err(e) => {
            ctx.reply(format!("Failed to retrieve issue overrides: {e}"))
                .await?;
            return Ok(());
        }
    };

    if overrides.len() == 0 {
        ctx.reply("Nobody has voted to override their issue yet.")
            .await?;
        return Ok(());
    }

    let mut message = format!(
        "**<@{}>'s issue:** {}\n**Override votes:** {}\n",
        message.author.id,
        issue_message.link(ctx.channel_id(), ctx.guild_id()),
        overrides.len()
    );
    for (override_author, override_message) in &overrides {
        let link = override_message.link(ctx.channel_id(), ctx.guild_id());
        message += &format!("- <@{override_author}>: {link}\n");
    }

    ctx.reply(message).await?;

    Ok(())
}
