use std::{sync::Arc, time::Duration};

use chrono::{Days, Utc};
use poise::{
    Modal, execute_modal_on_component_interaction,
    serenity_prelude::{
        ChannelId, ComponentInteraction, ComponentInteractionCollector,
        ComponentInteractionDataKind, CreateAllowedMentions, CreateForumPost,
        CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
        EditInteractionResponse,
    },
};
use sqlx::{Pool, Sqlite};
use tracing::error;

use crate::{
    discord::{
        content_review::data::{config::Config, discussions::DiscussionRecord},
        content_review::{
            BUTTON_ID_ACTION_NOT_NEEDED, BUTTON_ID_ACTION_START_PRIVATE,
            BUTTON_ID_ACTION_START_PUBLIC, INTERACTION_ID_PREFIX, create_pr_embed,
            discussion_channel_to_guild,
        },
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
    gh: Arc<GitHub>,
    intake_thread: ChannelId,
    private: bool,
) {
    async fn inner(
        interaction: ComponentInteraction,
        ctx: poise::serenity_prelude::Context,
        mut discussion: DiscussionRecord,
        db: Pool<Sqlite>,
        gh: Arc<GitHub>,
        intake_thread: ChannelId,
        private: bool,
    ) -> Result<(), Option<&'static str>> {
        let forum_channel = if private {
            Config::get_private_forum(&db)
                .await
                .ok_or(Some("Did you set up the private forum?"))?
        } else {
            Config::get_public_forum(&db)
                .await
                .ok_or(Some("Did you set up the public forum?"))?
        };

        let under_review_label = Config::get_under_review_label(&db).await.ok_or(Some(
            "Can't process review start with under review label unset.",
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
            None
        })?
        .ok_or(None)?;

        let review_time_days = review_settings
            .review_time_days
            .parse::<u64>()
            .map_err(|_| Some("Failed to parse review time."))?;

        if review_time_days > 90 {
            return Err(Some(
                "Invalid review time provided. It can't be longer than 90 days.",
            ));
        }

        let due_at = Utc::now()
            .checked_add_days(Days::new(review_time_days))
            .ok_or(None)?;

        gh.octo_install
            .issues(&gh.repo_owner, &gh.repo_name)
            .add_labels(discussion.pr_id, &[under_review_label])
            .await
            .map_err(|e| {
                error!(
                    "Failed to set under review label for PR #{}: {e}",
                    discussion.pr_id
                );
                Some("Failed to set under review label on GitHub.")
            })?;

        gh
            .octo_install
            .issues(&gh.repo_owner, &gh.repo_name)
            .create_comment(discussion.pr_id, format!(
                "**Triaged by {}:**\nThis PR requires a content review discussion, which will be held in {}.\n{}{}",
                interaction.user.name,
                if private { "private" } else { "public" },
                if let Some(reasoning) = review_settings.reasoning {
                    format!("```\n{reasoning}\n```\n")
                } else
                {
                    String::new()
                },
                format!("The review duration has been set to {review_time_days} days.")
            ))
            .await
            .map_err(|e| {
                    error!(
                    "Failed to comment about CR review on PR #{}: {e}",
                    discussion.pr_id
                    );
                    Some("Failed to add GitHub comment.")
                }
            )?;

        let mut message = CreateMessage::new().add_embeds(vec![
            create_pr_embed(
                discussion.pr_id,
                discussion.pr_title.clone(),
                discussion.pr_author.clone(),
                discussion.pr_body.clone(),
                &gh,
            )
            .field(
                "Review duration",
                format!("{} days", review_time_days),
                true,
            )
            .field("Due", format!("<t:{}:R>", due_at.timestamp()), true),
        ]);

        if let Some(ping_role) = Config::get_review_ping_role(&db).await {
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

                Some("Failed to create forum post.")
            })?;

        discussion
            .set_thread_id(&db, new_thread.id)
            .await
            .map_err(|()| None)?;

        discussion
            .setup_review_time(&db, review_time_days)
            .await
            .map_err(|()| None)?;

        intake_thread.delete(&ctx).await.map_err(|e| {
            error!(
                "Failed to delete intake discussion for pr {}: {e}",
                discussion.pr_id
            );
            Some("Failed to delete intake discussion.")
        })?;

        Ok(())
    }

    match inner(
        interaction.clone(),
        ctx.clone(),
        discussion,
        db,
        gh,
        intake_thread,
        private,
    )
    .await
    {
        Err(Some(e)) => {
            let _ = interaction
                .create_response(
                    &ctx,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new().content(format!("Error: {e}")),
                    ),
                )
                .await;
        }
        Err(None) => {
            let _ = interaction
                .create_response(
                    &ctx,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content(format!("An internal error occurred.")),
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
    gh: Arc<GitHub>,
    intake_thread: ChannelId,
) {
    async fn inner(
        interaction: ComponentInteraction,
        ctx: poise::serenity_prelude::Context,
        discussion: DiscussionRecord,
        db: Pool<Sqlite>,
        gh: Arc<GitHub>,
        intake_thread: ChannelId,
    ) -> Result<(), &'static str> {
        let no_review_needed_label = Config::get_no_review_needed_label(&db)
            .await
            .ok_or("Can't process No Review Needed with GitHub label unset.")?;

        discussion.delete(&db).await.map_err(|()| "Failed to delete discussion from DB. Can't process no review needed press further.")?;

        gh.octo_install
            .issues(&gh.repo_owner, &gh.repo_name)
            .add_labels(discussion.pr_id, &[no_review_needed_label])
            .await
            .map_err(|e| {
                error!(
                    "Failed to set no review needed label on PR #{}: {e}",
                    discussion.pr_id
                );
                "Failed to set GitHub label."
            })?;

        gh.octo_install
            .issues(&gh.repo_owner, &gh.repo_name)
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
                "Failed to create GitHub comment."
            })?;

        intake_thread.delete(&ctx).await.map_err(|e| {
            error!(
                "Failed to delete intake discussion for PR #{}: {e}",
                discussion.pr_id
            );
            "Failed to delete intake discussion."
        })?;

        Ok(())
    }

    match inner(
        interaction.clone(),
        ctx.clone(),
        discussion,
        db,
        gh,
        intake_thread,
    )
    .await
    {
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

// TODO: This should do as little work as possible to verify permissions and basic validity before
//       spawning a task to handle the interaction so other interactions aren't held up
pub async fn cr_component_task(
    ctx: poise::serenity_prelude::Context,
    db: Pool<Sqlite>,
    gh: Arc<GitHub>,
) {
    while let Some(interaction) = ComponentInteractionCollector::new(&ctx)
        .filter(move |i| {
            i.data
                .custom_id
                .starts_with(&format!("{INTERACTION_ID_PREFIX}_"))
        })
        .await
    {
        // TODO: Check permissions
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

                let Some(intake_forum) = Config::get_intake_forum(&db).await else {
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

                if parent_forum != intake_forum {
                    error!(
                        "Received button press {id_parts:?}, but parent forum was not intake forum."
                    );
                    let _ = interaction.create_response(&ctx, error_response).await;

                    continue;
                }

                let intake_thread = discussion.thread_id;

                match id_parts[1] {
                    BUTTON_ID_ACTION_START_PUBLIC => {
                        tokio::spawn(start_review_task(
                            interaction,
                            ctx.clone(),
                            discussion,
                            db.clone(),
                            gh.clone(),
                            intake_thread,
                            false,
                        ));
                    }

                    BUTTON_ID_ACTION_START_PRIVATE => {
                        tokio::spawn(start_review_task(
                            interaction,
                            ctx.clone(),
                            discussion,
                            db.clone(),
                            gh.clone(),
                            intake_thread,
                            true,
                        ));
                    }

                    BUTTON_ID_ACTION_NOT_NEEDED => {
                        tokio::spawn(no_review_needed_task(
                            interaction,
                            ctx.clone(),
                            discussion,
                            db.clone(),
                            gh.clone(),
                            intake_thread,
                        ));
                    }

                    action => {
                        error!("Received button press with invalid action {}", action);
                        let _ = interaction.create_response(&ctx, error_response).await;
                        continue;
                    }
                }
            }
            _ => {}
        }
    }
}
