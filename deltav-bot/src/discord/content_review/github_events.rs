use std::sync::Arc;

use poise::serenity_prelude::{
    ChannelId, CreateActionRow, CreateAllowedMentions, CreateButton, CreateEmbed,
    CreateEmbedAuthor, CreateForumPost, CreateMessage, EditThread, Mentionable,
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
            data::{
                config::{CrConfig, ignored::IgnoredKind},
                discussions::DiscussionRecord,
                forums::ForumRecord,
            },
            discussion_channel_to_guild,
        },
        pr_feeds::data::PrDashboards,
    },
    github::{GitHub, GitHubMessage},
};

const GH_COMMENT_COMMAND: &'static str = "!discord";
const GH_REVIEW_COMMAND: &'static str = "!review";

// TODO: spawn tasks to handle these, use semaphore
pub async fn cr_github_task(
    ctx: poise::serenity_prelude::Context,
    receiver: Arc<Mutex<Receiver<GitHubMessage>>>,
    db: Pool<Sqlite>,
    gh: Arc<GitHub>,
    config: CrConfig,
    pr_feeds: PrDashboards,
) {
    'outer: while let Some(message) = receiver.lock().await.recv().await {
        match message {
            GitHubMessage::PrOpened {
                pr_id,
                pr_title,
                pr_body,
                opened_by,
            } => {
                if config
                    .ignored
                    .is_ignored(IgnoredKind::Author, &opened_by)
                    .await
                {
                    info!("Ignoring opened PR #{pr_id} due to ignored author {opened_by}.");
                    continue;
                }

                let Some(intake_forum) = config.get_intake_forum().await else {
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

                let issues = gh.octo_install.issues_by_id(gh.repo);
                let labels = match issues
                    .list_labels_for_issue(pr_id)
                    .per_page(100)
                    .send()
                    .await
                {
                    Err(e) => {
                        error!("Failed to get list of labels for PR #{pr_id}: {e:#?}");
                        Vec::new() // better to start a duplicate thread than to ignore a new PR
                    }
                    Ok(x) => x.items,
                };

                let defined_labels = config.get_defined_github_labels().await;
                for label in &labels {
                    if defined_labels.contains(&label.name) {
                        info!(
                            "Not creating intake thread for opened PR with already applied CR label '{}'",
                            label.name
                        );
                        continue 'outer;
                    }
                }

                create_intake_post(
                    intake_forum,
                    &ctx,
                    pr_id,
                    pr_title,
                    opened_by,
                    pr_body,
                    &gh,
                    &db,
                )
                .await;
            }

            GitHubMessage::Comment {
                issue_id,
                username,
                comment,
                is_pr_author,
                is_staff: is_maintainer,
                is_contributor,
            } => {
                let comment_lower = comment.to_ascii_lowercase();
                if comment_lower.starts_with(GH_COMMENT_COMMAND) {
                    if !is_maintainer && !is_pr_author {
                        continue;
                    }

                    let Some(discussion) = DiscussionRecord::get_by_pr(&db, issue_id).await else {
                        continue;
                    };

                    relay_github_comment(username, issue_id, discussion, comment, &ctx).await;
                } else if comment_lower.trim() == GH_REVIEW_COMMAND {
                    if !is_pr_author && !is_contributor {
                        continue;
                    }

                    if DiscussionRecord::get_by_pr(&db, issue_id).await.is_some() {
                        info!(
                            "{username} tried to request a CR review for PR #{issue_id} using the GitHub command, but a discussion record already exists."
                        );
                        continue;
                    };

                    let Some(intake_forum) = config.get_intake_forum().await else {
                        continue;
                    };

                    let pr = match gh
                        .octo_install
                        .pulls(&gh.repo_owner, &gh.repo_name)
                        .get(issue_id)
                        .await
                    {
                        Ok(x) => x,
                        Err(e) => {
                            error!(
                                "{username} requested a CR review for on issue ID {issue_id}, but PR data could not be fetched: {e:#?}"
                            );
                            continue;
                        }
                    };

                    create_intake_post(
                        intake_forum,
                        &ctx,
                        issue_id,
                        pr.title.unwrap_or("Untitled".into()),
                        pr.user
                            .and_then(|x| Some(x.login))
                            .unwrap_or("Unknown".into()),
                        pr.body,
                        &gh,
                        &db,
                    )
                    .await;
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

                if discussion.forum_id == config.get_intake_forum().await.unwrap_or_default() {
                    info!("PR #{pr_id} is still in intake after closure. Deleting discussion.");

                    if let Err(e) = discussion.thread_id.delete(&ctx).await {
                        error!("Failed to delete thread for PR #{pr_id}: {e:#?}");
                    }

                    let _ = discussion.delete(&db).await;
                    continue;
                }

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
                        EditThread::new()
                            .applied_tags(
                                guild_channel
                                    .applied_tags
                                    .iter()
                                    .chain([forum.tag_pr_closed].iter())
                                    .cloned(),
                            )
                            .archived(true),
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

                if discussion.forum_id == config.get_intake_forum().await.unwrap_or_default() {
                    info!("PR #{pr_id} is still in intake after merge. Deleting discussion.");

                    if let Err(e) = discussion.thread_id.delete(&ctx).await {
                        error!("Failed to delete thread for PR #{pr_id}: {e:#?}");
                    }

                    let _ = discussion.delete(&db).await;
                    continue;
                }

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
                        EditThread::new()
                            .applied_tags(
                                guild_channel
                                    .applied_tags
                                    .iter()
                                    .chain([forum.tag_pr_merged].iter())
                                    .cloned(),
                            )
                            .archived(true),
                    )
                    .await
                {
                    error!(
                        "Failed to add merged tag to {:?}: {e:#?}",
                        discussion.thread_id
                    );
                }
            }

            GitHubMessage::PrDrafted { pr_id, drafted_by } => {
                let Some(discussion) = DiscussionRecord::get_by_pr(&db, pr_id).await else {
                    continue;
                };

                if let Err(e) = discussion
                    .thread_id
                    .send_message(
                        &ctx,
                        CreateMessage::new().content(format!(
                            "This PR has been converted into a draft by `{drafted_by}`."
                        )),
                    )
                    .await
                {
                    error!("Failed to send message about PR {pr_id} closing: {e:#?}");
                }
            }

            GitHubMessage::PrLabeled { pr_id, label } => {
                if let Some(discussion) = DiscussionRecord::get_by_pr(&db, pr_id).await {
                    if config.ignored.is_ignored(IgnoredKind::Label, &label).await
                        && discussion.forum_id
                            == config.get_intake_forum().await.unwrap_or_default()
                    {
                        info!(
                            "PR #{pr_id} has received ignored label '{label}' and is still in CR intake forum. Deleting it."
                        );

                        if let Err(e) = discussion.thread_id.delete(&ctx).await {
                            error!("Failed to delete discussion thread for PR #{pr_id}: {e:#?}");
                        }

                        let _ = discussion.delete(&db).await;

                        if let Err(e) = gh.octo_install.issues_by_id(gh.repo).create_comment(pr_id, format!("This PR has been automatically excluded from the Content Review process because it was labeled `{label}`. If it should be reviewed anyway, please comment `!review`.")).await {
                            error!("Failed to create comment about PR #{pr_id} being ignored for label {label}: {e:#?}");
                        }
                    }
                }

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

async fn relay_github_comment(
    username: String,
    issue_id: u64,
    discussion: DiscussionRecord,
    comment: String,
    ctx: &poise::serenity_prelude::Context,
) {
    info!(
        "Author or maintainer {username} wrote a comment in PR #{issue_id}, associated with thread {}.",
        discussion.thread_id.get()
    );

    let comment: String = comment[GH_COMMENT_COMMAND.len()..]
        .chars()
        .take(EMBED_DESC_MAX_LEN)
        .collect();

    if let Err(e) = discussion
        .thread_id
        .send_message(
            ctx,
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

async fn create_intake_post(
    intake_forum: ChannelId,
    ctx: &poise::serenity_prelude::Context,
    pr_id: u64,
    pr_title: String,
    opened_by: String,
    pr_body: Option<String>,
    gh: &Arc<GitHub>,
    db: &Pool<Sqlite>,
) {
    match intake_forum
        .create_forum_post(
            ctx,
            CreateForumPost::new(
                format!("{pr_title} #{pr_id}"),
                CreateMessage::new()
                    .add_embed(create_pr_embed(
                        pr_id,
                        pr_title.clone(),
                        opened_by.clone(),
                        pr_body.clone(),
                        gh,
                    ))
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
                forum_id: intake_forum,
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
