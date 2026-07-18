use poise::serenity_prelude::{
    ComponentInteraction, CreateInteractionResponse, CreateInteractionResponseFollowup,
};
use sqlx::{Pool, Sqlite};

use crate::discord::{
    HandledError,
    content_review::{
        data::discussions::DiscussionRecord, raised_issues::create_issue_overview_embeds,
    },
};

pub async fn button_click_view_issues_task(
    interaction: ComponentInteraction,
    ctx: poise::serenity_prelude::Context,
    pr_id: u64,
    db: Pool<Sqlite>,
) {
    async fn inner(
        interaction: &ComponentInteraction,
        ctx: &poise::serenity_prelude::Context,
        pr_id: u64,
        db: Pool<Sqlite>,
    ) -> Result<(), HandledError> {
        let Some(discussion) = DiscussionRecord::get_by_pr(&db, pr_id).await else {
            return Err(HandledError::InternalError);
        };

        let _ = interaction
            .create_response(ctx, CreateInteractionResponse::Acknowledge)
            .await;

        // EMBEDS
        let mut embeds = create_issue_overview_embeds(ctx, &db, &discussion).await?;

        if embeds.len() == 0 {
            let _ = interaction
                .create_followup(
                    &ctx,
                    CreateInteractionResponseFollowup::new()
                        .ephemeral(true)
                        .content(
                            "There are no issues or overrides associated with this discussion.",
                        ),
                )
                .await;
            return Ok(());
        }

        let mut embeds = embeds.drain(..);
        let message_default = CreateInteractionResponseFollowup::new().ephemeral(true);
        let mut message = message_default.clone();
        let mut message_embeds = 0;

        while let Some(embed) = embeds.next() {
            if message_embeds == 10 {
                let _ = interaction.create_followup(&ctx, message).await;
                message = message_default.clone();
                message_embeds = 0;
            }

            message = message.add_embed(embed);
            message_embeds += 1;
        }

        if message_embeds != 0 {
            let _ = interaction.create_followup(&ctx, message).await;
        }

        Ok(())
    }

    match inner(&interaction, &ctx, pr_id, db).await {
        Err(e) => {
            let _ = interaction
                .create_followup(
                    &ctx,
                    CreateInteractionResponseFollowup::new()
                        .ephemeral(true)
                        .content(format!("Error: {e}")),
                )
                .await;
        }
        Ok(()) => (),
    }
}
