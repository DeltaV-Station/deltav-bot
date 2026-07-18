use std::sync::Arc;

use poise::serenity_prelude::{
    ComponentInteractionCollector, ComponentInteractionDataKind, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use sqlx::{Pool, Sqlite};
use tracing::{error, info};

use crate::{
    discord::{
        content_review::{
            consts::{
                BUTTON_ID_ACTION_MUTE_REMINDERS, BUTTON_ID_ACTION_NOT_NEEDED,
                BUTTON_ID_ACTION_START_PRIVATE, BUTTON_ID_ACTION_START_PUBLIC,
                BUTTON_ID_ACTION_VIEW_ISSUES, INTERACTION_ID_PREFIX,
            },
            data::config::CrConfig,
            raised_issues::comp_tasks::button_click_view_issues_task,
            triage::comp_tasks::{
                button_click_mute_reminders_task, button_click_no_review_needed_task,
                start_review_task,
            },
        },
        permissions::{
            check_permissions_component,
            data::{PermissionFlags, Permissions},
        },
    },
    github::GitHub,
};

// needed to call poise functions that expect to take a poise context from the task
pub struct CtxWrapper<'a> {
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

                info!(
                    "{} ({}) has pressed button {id_parts:?}",
                    interaction.user.name, interaction.user.id
                );

                match id_parts[1] {
                    BUTTON_ID_ACTION_START_PUBLIC => {
                        tokio::spawn(start_review_task(
                            interaction,
                            ctx.clone(),
                            pr_id,
                            db.clone(),
                            config.clone(),
                            gh.clone(),
                            false,
                        ));
                    }

                    BUTTON_ID_ACTION_START_PRIVATE => {
                        tokio::spawn(start_review_task(
                            interaction,
                            ctx.clone(),
                            pr_id,
                            db.clone(),
                            config.clone(),
                            gh.clone(),
                            true,
                        ));
                    }

                    BUTTON_ID_ACTION_NOT_NEEDED => {
                        tokio::spawn(button_click_no_review_needed_task(
                            interaction,
                            ctx.clone(),
                            pr_id,
                            db.clone(),
                            config.clone(),
                            gh.clone(),
                        ));
                    }

                    BUTTON_ID_ACTION_MUTE_REMINDERS => {
                        tokio::spawn(button_click_mute_reminders_task(
                            interaction,
                            ctx.clone(),
                            db.clone(),
                            pr_id,
                        ));
                    }

                    BUTTON_ID_ACTION_VIEW_ISSUES => {
                        tokio::spawn(button_click_view_issues_task(
                            interaction,
                            ctx.clone(),
                            pr_id,
                            db.clone(),
                        ));
                    }

                    _ => continue,
                }
            }
            _ => (),
        }
    }
}
