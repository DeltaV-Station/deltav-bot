use std::{sync::Arc, time::Duration};

use chrono::{Days, Utc};
use poise::{
    Modal, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, ComponentInteractionDataKind,
        CreateAllowedMentions, CreateEmbed, CreateForumPost, CreateInteractionResponse,
        CreateInteractionResponseMessage, CreateMessage, EditInteractionResponse,
    },
};
use sqlx::{Pool, Sqlite};
use tracing::{error, info};

use crate::{
    discord::{
        content_review::{
            BUTTON_ID_ACTION_MUTE_REMINDERS, BUTTON_ID_ACTION_NOT_NEEDED,
            BUTTON_ID_ACTION_START_PRIVATE, BUTTON_ID_ACTION_START_PUBLIC, HandledError,
            INTERACTION_ID_PREFIX, create_pr_embed,
            data::{config::CrConfig, discussions::DiscussionRecord, forums::ForumRecord},
            discussion_channel_to_guild,
        },
        permissions::{
            check_permissions_component,
            data::{PermissionFlags, Permissions},
        },
        to_md_quote_block,
    },
    github::GitHub,
};

#[derive(Debug, Modal)]
#[name = "Start a review"] // Struct name by default
struct StartReviewModal {
    #[name = "Review time (days)"]
    #[placeholder = "for example: 7"]
    #[min_length = 1]
    #[max_length = 2]
    review_time_days: String,
    #[name = "Reasoning"]
    #[placeholder = "Why does this require a public/private review? This can be left empty."] // No placeholder by default
    #[paragraph]
    reasoning: Option<String>,
}

// needed to call poise functions that expect to take a poise context from the task
struct CtxWrapper<'a> {
    context: &'a poise::serenity_prelude::Context,
}

impl<'a> CtxWrapper<'a> {
    pub fn new(context: &'a poise::serenity_prelude::Context) -> Self {
        Self { context }
    }
}

impl<'a> AsRef<poise::serenity_prelude::Context> for CtxWrapper<'a> {
    fn as_ref(&self) -> &poise::serenity_prelude::Context {
        self.context
    }
}

pub async fn start_review_task(
    interaction: ComponentInteraction,
    ctx: poise::serenity_prelude::Context,
    discussion: DiscussionRecord,
    db: Pool<Sqlite>,
    config: CrConfig,
    gh: Arc<GitHub>,
    private: bool,
) {
    async fn inner(
        interaction: &ComponentInteraction,
        ctx: &poise::serenity_prelude::Context,
        mut discussion: DiscussionRecord,
        db: Pool<Sqlite>,
        config: CrConfig,
        gh: Arc<GitHub>,
        private: bool,
    ) -> Result<(), HandledError> {
        let forum_channel = if private {
            config
                .get_private_forum()
                .await
                .ok_or(HandledError::UserfacingError(
                    "Can't process review start with private forum unset.".into(),
                ))?
        } else {
            config
                .get_public_forum()
                .await
                .ok_or(HandledError::UserfacingError(
                    "Can't process review start with public forum unset.".into(),
                ))?
        };

        let under_review_label =
            config
                .get_under_review_label()
                .await
                .ok_or(HandledError::UserfacingError(
                    "Can't process review start with under review label unset.".into(),
                ))?;

        let review_settings = execute_modal_on_component_interaction::<StartReviewModal>(
            CtxWrapper::new(&ctx),
            interaction.clone(),
            None,
            Some(Duration::from_mins(60)),
        )
        .await
        .map_err(|e| {
            error!("Failed to execute review settings modal: {e}");
            HandledError::InternalError
        })?
        .ok_or(HandledError::UserfacingError("test".into()))?;

        let review_time_days = review_settings
            .review_time_days
            .parse::<u64>()
            .map_err(|_| HandledError::UserfacingError("Failed to parse review time.".into()))?;

        if review_time_days > 90 {
            return Err(HandledError::UserfacingError(
                "Invalid review time provided. It can't be longer than 90 days.".into(),
            ));
        }

        let due_at = Utc::now()
            .checked_add_days(Days::new(review_time_days))
            .ok_or(HandledError::InternalError)?;

        let issues = gh.octo_install.issues_by_id(gh.repo);

        issues
            .add_labels(discussion.pr_id, &[under_review_label])
            .await
            .map_err(|e| {
                error!(
                    "Failed to set under review label for PR #{}: {e}",
                    discussion.pr_id
                );

                return HandledError::UserfacingError(
                    "Failed to set under review label on GitHub.".into(),
                );
            })?;

        let mut message = CreateMessage::new().add_embeds(vec![
            create_pr_embed(
                discussion.pr_id,
                discussion.pr_title.clone(),
                discussion.pr_author.clone(),
                discussion.pr_body.clone(),
                &gh,
            ),
            CreateEmbed::new()
                .title(format!("Triaged by {}", interaction.user.name))
                .description(
                    review_settings
                        .reasoning
                        .clone()
                        .unwrap_or("*No reasoning provided.*".into()),
                )
                .field(
                    "Review duration",
                    format!("{} days", review_time_days),
                    true,
                )
                .field("Due", format!("<t:{}:R>", due_at.timestamp()), true),
        ]);

        if let Some(ping_role) = config.get_review_ping_role().await {
            message = message
                .allowed_mentions(CreateAllowedMentions::new().roles([ping_role]))
                .content(format!("<@&{}>", ping_role.get()));
        }

        let new_thread = forum_channel
            .create_forum_post(
                &ctx,
                CreateForumPost::new(
                    format!("{} #{}", discussion.pr_title, discussion.pr_id),
                    message,
                ),
            )
            .await
            .map_err(|e| {
                error!(
                    "Failed to create forum post to start review of PR #{}: {e}",
                    discussion.pr_id
                );

                HandledError::UserfacingError("Failed to create forum post.".into())
            })?;

        issues
            .create_comment(
                discussion.pr_id,
                format!(
                    r#"**Triaged by {}:**
This PR requires a content review discussion, which will be held in {}.
{}
You can [view the discussion here]({}) and write comments starting with `!discord` to send messages into the thread.
{}"#,
                    interaction.user.name,
                    if private { "private" } else { "public" },
                    if let Some(reasoning) = review_settings.reasoning {
                        to_md_quote_block(reasoning)
                    } else {
                        String::new()
                    },
                    format!("https://discord.com/channels/{}/{}", new_thread.guild_id.get(), new_thread.id.get()),
                    format!("The review duration has been set to {review_time_days} days."),
                ),
            )
            .await
            .map_err(|e| {
                error!(
                    "Failed to comment about CR review on PR #{}: {e}",
                    discussion.pr_id
                );

                return HandledError::UserfacingError("Failed to add GitHub comment.".into());
            })?;

        let intake_thread = discussion.thread_id;

        discussion.set_forum_id(&db, forum_channel).await?;
        discussion.set_thread_id(&db, new_thread.id).await?;
        discussion.delete_body(&db).await?;
        discussion.setup_review_time(&db, review_time_days).await?;

        intake_thread.delete(&ctx).await.map_err(|e| {
            error!(
                "Failed to delete intake discussion for pr {}: {e}",
                discussion.pr_id
            );
            HandledError::UserfacingError("Failed to delete intake discussion.".into())
        })?;

        Ok(())
    }

    match inner(&interaction, &ctx, discussion, db, config, gh, private).await {
        Err(e) => {
            let _ = interaction
                .create_response(
                    &ctx,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new().content(format!("Error: {e}")),
                    ),
                )
                .await;
        }
        Ok(()) => (),
    }
}

pub async fn no_review_needed_task(
    interaction: ComponentInteraction,
    ctx: poise::serenity_prelude::Context,
    discussion: DiscussionRecord,
    db: Pool<Sqlite>,
    config: CrConfig,
    gh: Arc<GitHub>,
) {
    async fn inner(
        interaction: &ComponentInteraction,
        ctx: &poise::serenity_prelude::Context,
        discussion: DiscussionRecord,
        db: Pool<Sqlite>,
        config: CrConfig,
        gh: Arc<GitHub>,
    ) -> Result<(), HandledError> {
        let no_review_needed_label =
            config
                .get_no_review_needed_label()
                .await
                .ok_or(HandledError::UserfacingError(
                    "Can't process No Review Needed with GitHub label unset.".into(),
                ))?;

        discussion.delete(&db).await?;

        let issues = gh.octo_install.issues_by_id(gh.repo);

        issues
            .add_labels(discussion.pr_id, &[no_review_needed_label])
            .await
            .map_err(|e| {
                error!(
                    "Failed to set no review needed label on PR #{}: {e}",
                    discussion.pr_id
                );
                HandledError::UserfacingError("Failed to set GitHub label.".into())
            })?;

        issues
            .create_comment(
                discussion.pr_id,
                format!(
                    "**Triaged by {}:** This PR does not require a content review discussion.",
                    interaction.user.name
                ),
            )
            .await
            .map_err(|e| {
                error!(
                    "Failed to create no review needed comment on PR #{}: {e}",
                    discussion.pr_id
                );
                HandledError::UserfacingError("Failed to create GitHub comment.".into())
            })?;

        discussion.thread_id.delete(&ctx).await.map_err(|e| {
            error!(
                "Failed to delete intake discussion for PR #{}: {e}",
                discussion.pr_id
            );
            HandledError::UserfacingError("Failed to delete intake discussion.".into())
        })?;

        Ok(())
    }

    match inner(&interaction, &ctx, discussion, db, config, gh).await {
        Err(e) => {
            let _ = interaction
                .create_response(
                    &ctx,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new().content(format!("Error: {e}")),
                    ),
                )
                .await;
        }
        Ok(()) => (),
    }
}

// TODO: Use semaphore
pub async fn cr_component_task(
    ctx: poise::serenity_prelude::Context,
    db: Pool<Sqlite>,
    gh: Arc<GitHub>,
    permissions: Permissions,
    config: CrConfig,
) {
    while let Some(interaction) = ComponentInteractionCollector::new(&ctx)
        .filter(move |i| {
            i.data
                .custom_id
                .starts_with(&format!("{INTERACTION_ID_PREFIX}_"))
        })
        .await
    {
        match interaction.data.kind {
            ComponentInteractionDataKind::Button => {
                let error_response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new().content("An internal error occurred."),
                );

                let id_parts: Vec<&str> = interaction.data.custom_id.split("_").collect();
                if id_parts.len() != 3 {
                    error!("Received invalid button press with ID {id_parts:?}.");
                    let _ = interaction.create_response(&ctx, error_response).await;

                    continue;
                }

                let Ok(pr_id) = id_parts[2].parse::<u64>() else {
                    error!(
                        "Received invalid button press with pr_id='{}' ({id_parts:?}).",
                        id_parts[2]
                    );
                    let _ = interaction.create_response(&ctx, error_response).await;

                    continue;
                };

                let Some(discussion) = DiscussionRecord::get_by_pr(&db, pr_id).await else {
                    error!("Received button press {id_parts:?}, but could not find discussion.");
                    let _ = interaction.create_response(&ctx, error_response).await;

                    continue;
                };

                let check = check_permissions_component(
                    &ctx,
                    &interaction,
                    &permissions,
                    PermissionFlags::CONTENT_REVIEWER,
                )
                .await;
                if check.is_err() || !check.unwrap() {
                    continue;
                }

                let Some(parent_forum) =
                    discussion_channel_to_guild(pr_id, discussion.thread_id, &ctx)
                        .await
                        .and_then(|x| x.parent_id)
                else {
                    error!(
                        "Failed to get parent forum for discussion thread {}",
                        discussion.thread_id
                    );
                    let _ = interaction.create_response(&ctx, error_response).await;

                    continue;
                };

                let Some(intake_forum) = config.get_intake_forum().await else {
                    error!("Can't process interaction without intake forum.");
                    let _ = interaction
                        .edit_response(
                            &ctx,
                            EditInteractionResponse::new()
                                .content("Can't process CR interaction with intake forum unset."),
                        )
                        .await;

                    continue;
                };

                match id_parts[1] {
                    BUTTON_ID_ACTION_START_PUBLIC => {
                        if parent_forum != intake_forum {
                            error!(
                                "Received button press {id_parts:?}, but parent forum was not intake forum."
                            );
                            let _ = interaction.create_response(&ctx, error_response).await;

                            continue;
                        }

                        tokio::spawn(start_review_task(
                            interaction,
                            ctx.clone(),
                            discussion,
                            db.clone(),
                            config.clone(),
                            gh.clone(),
                            false,
                        ));
                    }

                    BUTTON_ID_ACTION_START_PRIVATE => {
                        if parent_forum != intake_forum {
                            error!(
                                "Received button press {id_parts:?}, but parent forum was not intake forum."
                            );
                            let _ = interaction.create_response(&ctx, error_response).await;

                            continue;
                        }

                        tokio::spawn(start_review_task(
                            interaction,
                            ctx.clone(),
                            discussion,
                            db.clone(),
                            config.clone(),
                            gh.clone(),
                            true,
                        ));
                    }

                    BUTTON_ID_ACTION_NOT_NEEDED => {
                        if parent_forum != intake_forum {
                            error!(
                                "Received button press {id_parts:?}, but parent forum was not intake forum."
                            );
                            let _ = interaction.create_response(&ctx, error_response).await;

                            continue;
                        }

                        tokio::spawn(no_review_needed_task(
                            interaction,
                            ctx.clone(),
                            discussion,
                            db.clone(),
                            config.clone(),
                            gh.clone(),
                        ));
                    }

                    BUTTON_ID_ACTION_MUTE_REMINDERS => {
                        if let None = ForumRecord::get_by_channel(&db, parent_forum).await {
                            error!(
                                "Received button press {id_parts:?}, but parent forum was not registered."
                            );
                            let _ = interaction.create_response(&ctx, error_response).await;
                            continue;
                        }

                        tokio::spawn(mute_reminders_task(
                            interaction,
                            ctx.clone(),
                            discussion,
                            db.clone(),
                        ));
                    }

                    _ => continue,
                }
            }
            _ => {}
        }
    }
}

async fn mute_reminders_task(
    interaction: ComponentInteraction,
    ctx: poise::serenity_prelude::Context,
    mut discussion: DiscussionRecord,
    db: Pool<Sqlite>,
) {
    match discussion.disable_reminders(&db).await {
        Ok(was_active) => {
            if !was_active {
                let _ = interaction
                    .create_response(
                        &ctx,
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .ephemeral(true)
                                .content("Reminders have already been disabled for this PR."),
                        ),
                    )
                    .await;
                return;
            }
            let message = format!(
                "{} disabled reminders for PR #{}.",
                interaction.user.name, discussion.pr_id
            );

            info!(message);

            let _ = interaction
                .create_response(
                    &ctx,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new().content(message),
                    ),
                )
                .await;
        }
        Err(e) => {
            let _ = interaction.defer_ephemeral(&ctx).await;
            let _ = interaction
                .create_response(
                    &ctx,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new().content(format!(
                            "Failed to disable reminders for PR #{}: {e}",
                            discussion.pr_id
                        )),
                    ),
                )
                .await;
        }
    }
}
