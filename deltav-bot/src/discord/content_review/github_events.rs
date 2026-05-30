use std::sync::Arc;

use poise::serenity_prelude::{
    CreateActionRow, CreateAllowedMentions, CreateButton, CreateEmbed, CreateEmbedAuthor,
    CreateForumPost, CreateMessage, EditThread, Mentionable,
};
use sqlx::{Pool, Sqlite};
use tokio::sync::{Mutex, mpsc::Receiver};
use tracing::{error, info, warn};

use crate::{
    consts::HTML_COMMENT_REGEX,
    discord::{
        EMBED_DESC_MAX_LEN,
        content_review::{
            BUTTON_ID_ACTION_NOT_NEEDED, BUTTON_ID_ACTION_START_PRIVATE,
            BUTTON_ID_ACTION_START_PUBLIC, INTERACTION_ID_PREFIX, create_pr_embed,
            data::{config::CrConfig, discussions::DiscussionRecord, forums::ForumRecord},
            discussion_channel_to_guild,
        },
        pr_feeds::data::PrFeeds,
    },
    github::{GitHub, GitHubMessage},
};

const GH_COMMENT_PREFIX: &'static str = "!discord";

pub async fn cr_github_task(
    ctx: poise::serenity_prelude::Context,
    receiver: Arc<Mutex<Receiver<GitHubMessage>>>,
    db: Pool<Sqlite>,
    gh: Arc<GitHub>,
    config: CrConfig,
    pr_feeds: PrFeeds,
) {
    while let Some(message) = receiver.lock().await.recv().await {
        match message {
            GitHubMessage::PrOpened {
                pr_id,
                pr_title,
                pr_body,
                opened_by,
            } => {
                let Some(main_forum) = config.get_intake_forum().await else {
                    warn!("Received PrOpened but main forum is not set.");
                    continue;
                };

                if let Some(discussion) = DiscussionRecord::get_by_pr(&db, pr_id).await {
                    let Some(forum) = ForumRecord::get_by_channel(&db, discussion.forum_id).await
                    else {
                        error!(
                            "Discussion for {pr_id} exists, but the forum does not have a record!"
                        );
                        continue;
                    };

                    if let Err(e) = discussion
                        .thread_id
                        .send_message(
                            &ctx,
                            CreateMessage::new()
                                .content(format!("This PR has been opened by `{opened_by}`.")),
                        )
                        .await
                    {
                        error!("Failed to send message about PR {pr_id} opening: {e}");
                    }

                    let Some(guild_channel) =
                        discussion_channel_to_guild(pr_id, discussion.thread_id, &ctx).await
                    else {
                        continue;
                    };

                    if let Err(e) = discussion
                        .thread_id
                        .edit_thread(
                            &ctx,
                            EditThread::new().applied_tags(
                                guild_channel
                                    .applied_tags
                                    .iter()
                                    .filter(|x| **x != forum.tag_pr_closed)
                                    .cloned(),
                            ),
                        )
                        .await
                    {
                        error!(
                            "Failed to remove closed tag from {:?}: {e:#?}",
                            discussion.thread_id
                        );
                    }

                    continue;
                }

                match main_forum
                    .create_forum_post(
                        &ctx,
                        CreateForumPost::new(
                            format!("{pr_title} #{pr_id}"),
                            CreateMessage::new()
                                .add_embed(create_pr_embed(pr_id, pr_title.clone(), opened_by.clone(), pr_body.clone(), &gh))
                                .components(vec![CreateActionRow::Buttons(vec![
                                    CreateButton::new(format!(
                                        "{INTERACTION_ID_PREFIX}_{BUTTON_ID_ACTION_START_PUBLIC}_{pr_id}"
                                    ))
                                    .label("Public review"),
                                    CreateButton::new(format!(
                                        "{INTERACTION_ID_PREFIX}_{BUTTON_ID_ACTION_START_PRIVATE}_{pr_id}"
                                    ))
                                    .label("Private review"),
                                    CreateButton::new(format!(
                                        "{INTERACTION_ID_PREFIX}_{BUTTON_ID_ACTION_NOT_NEEDED}_{pr_id}"
                                    ))
                                    .label("No review needed"),
                                ])]),
                        ),
                    )
                    .await
                {
                    Ok(post_channel) => {
                        info!("Created thread {} for PR #{pr_id}", post_channel.id.get());
                        let discussion = DiscussionRecord {
                            forum_id: main_forum,
                            pr_id,
                            thread_id: post_channel.id,
                            pr_title,
                            pr_author: opened_by,
                            pr_body,
                            ..Default::default()
                        };

                        let _ = discussion.insert(&db).await;
                    }
                    Err(e) => {
                        error!("Failed to create forum post for opened PR {pr_id}: {e:#?}");
                    }
                }
            }

            GitHubMessage::AuthorOrMaintCommented {
                issue_id,
                username,
                comment,
            } => {
                let Some(discussion) = DiscussionRecord::get_by_pr(&db, issue_id).await else {
                    continue;
                };

                if !comment.to_ascii_lowercase().starts_with(GH_COMMENT_PREFIX) {
                    continue;
                }

                info!(
                    "Author or maintainer {username} wrote a comment in PR #{issue_id}, associated with thread {}.",
                    discussion.thread_id.get()
                );

                let comment: String = comment[GH_COMMENT_PREFIX.len()..]
                    .chars()
                    .take(EMBED_DESC_MAX_LEN)
                    .collect();

                if let Err(e) = discussion
                    .thread_id
                    .send_message(
                        &ctx,
                        CreateMessage::new().add_embed(
                            CreateEmbed::new()
                                .author(CreateEmbedAuthor::new(format!("{username} via GitHub")))
                                .description(comment),
                        ),
                    )
                    .await
                {
                    error!(
                        "Failed to relay GitHub comment for PR #{issue_id} to Discord thread {}: {e:#?}",
                        discussion.thread_id
                    );
                }
            }

            GitHubMessage::PrClosed { pr_id, closed_by } => {
                let Some(discussion) = DiscussionRecord::get_by_pr(&db, pr_id).await else {
                    continue;
                };
                info!(
                    "PR {pr_id}, associated with thread {}, has been closed.",
                    discussion.thread_id.get()
                );

                let Some(forum) = ForumRecord::get_by_channel(&db, discussion.forum_id).await
                else {
                    continue;
                };

                let Some(guild_channel) =
                    discussion_channel_to_guild(pr_id, discussion.thread_id, &ctx).await
                else {
                    continue;
                };

                if let Err(e) = guild_channel
                    .send_message(
                        &ctx,
                        CreateMessage::new()
                            .content(format!("This PR has been closed by `{closed_by}`.")),
                    )
                    .await
                {
                    error!("Failed to send message about PR {pr_id} closing: {e:#?}");
                }

                if let Err(e) = discussion
                    .thread_id
                    .edit_thread(
                        &ctx,
                        EditThread::new().applied_tags(
                            guild_channel
                                .applied_tags
                                .iter()
                                .chain([forum.tag_pr_closed].iter())
                                .cloned(),
                        ),
                    )
                    .await
                {
                    error!(
                        "Failed to add closed tag to {:?}: {e:#?}",
                        discussion.thread_id
                    );
                }
            }

            GitHubMessage::PrMerged { pr_id, merged_by } => {
                let Some(discussion) = DiscussionRecord::get_by_pr(&db, pr_id).await else {
                    continue;
                };
                info!(
                    "PR {pr_id}, associated with thread {}, has been merged.",
                    discussion.thread_id.get()
                );

                let Some(forum) = ForumRecord::get_by_channel(&db, discussion.forum_id).await
                else {
                    continue;
                };

                let Some(guild_channel) =
                    discussion_channel_to_guild(pr_id, discussion.thread_id, &ctx).await
                else {
                    continue;
                };

                if let Err(e) = guild_channel
                    .send_message(
                        &ctx,
                        CreateMessage::new()
                            .content(format!("This PR has been merged by `{merged_by}`.")),
                    )
                    .await
                {
                    error!("Failed to send message about PR {pr_id} being merged: {e:#?}");
                }

                if let Err(e) = discussion
                    .thread_id
                    .edit_thread(
                        &ctx,
                        EditThread::new().applied_tags(
                            guild_channel
                                .applied_tags
                                .iter()
                                .chain([forum.tag_pr_merged].iter())
                                .cloned(),
                        ),
                    )
                    .await
                {
                    error!(
                        "Failed to add merged tag to {:?}: {e:#?}",
                        discussion.thread_id
                    );
                }
            }

            GitHubMessage::PrLabeled { pr_id, label } => {
                let Some(feed) = pr_feeds.get_by_label(&label).await else {
                    continue;
                };

                let issue = match gh.octo_install.issues_by_id(gh.repo).get(pr_id).await {
                    Ok(x) => x,
                    Err(e) => {
                        error!(
                            "Failed to fetch issue metadata to post about PR #{pr_id} in PrFeed {}: {e:#?}",
                            feed.id
                        );
                        continue;
                    }
                };

                let mut message = CreateMessage::new().embed(create_pr_embed(
                    pr_id,
                    issue.title,
                    issue.user.login,
                    issue
                        .body
                        .and_then(|x| Some(HTML_COMMENT_REGEX.replace_all(&x, "").to_string())),
                    &gh,
                ));

                if let Some(ping_role) = feed.ping_role {
                    message = message
                        .content(ping_role.mention().to_string())
                        .allowed_mentions(CreateAllowedMentions::new().roles(&[ping_role]));
                }

                if let Err(e) = feed.channel_id.send_message(&ctx, message).await {
                    error!(
                        "Failed to send PR feed for PR #{pr_id} message to channel {}: {e}",
                        feed.channel_id
                    );
                }
            }
        }
    }
}
