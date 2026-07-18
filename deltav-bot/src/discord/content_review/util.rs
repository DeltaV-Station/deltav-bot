use poise::serenity_prelude::{ChannelId, CreateEmbed, CreateEmbedFooter, GuildChannel};
use tracing::error;

use crate::{
    discord::{
        Context, EMBED_DESC_MAX_LEN, Error, HandledError,
        content_review::data::{discussions::DiscussionRecord, forums::ForumRecord},
    },
    github::GitHub,
};

pub async fn discussion_channel_to_guild(
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

pub async fn try_remove_label(
    gh: &GitHub,
    label: impl Into<String>,
    ctx: &Context<'_>,
    discussion: &DiscussionRecord,
) -> Result<(), Error> {
    let label = label.into();

    if let Err(e) = gh
        .octo_install
        .issues_by_id(gh.repo)
        .remove_label(discussion.pr_id, &label)
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
                "Failed to remove '{}' label from PR #{}: {e:#?}",
                label, discussion.pr_id
            );

            ctx.reply(format!("Failed to remove '{label}' label"))
                .await?;
        }
    }

    Ok(())
}

pub async fn get_channel_discussion(
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
