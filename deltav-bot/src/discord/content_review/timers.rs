use std::time::Duration;

use chrono::Utc;
use poise::serenity_prelude::{
    CreateActionRow, CreateAllowedMentions, CreateButton, CreateEmbed, CreateMessage,
};
use sqlx::{Pool, Sqlite};
use tokio::time::sleep;
use tracing::{error, info};

use crate::discord::content_review::{
    BUTTON_ID_ACTION_MUTE_REMINDERS, BUTTON_ID_ACTION_VIEW_ISSUES, HandledError,
    INTERACTION_ID_PREFIX,
    data::{config::CrConfig, discussions::DiscussionRecord},
};

pub async fn cr_timers_task(
    ctx: poise::serenity_prelude::Context,
    db: Pool<Sqlite>,
    config: CrConfig,
) {
    loop {
        let discussions = DiscussionRecord::get_all(&db).await;

        for mut discussion in discussions {
            let Some(next_day_micros) = discussion.review_days_next_micros else {
                continue;
            };

            if next_day_micros > Utc::now().timestamp_micros() {
                continue;
            }

            if let Err(HandledError::InternalError) = discussion.advance_review_timer(&db).await {
                error!("Failed to advance review timer for {discussion:?}");
                continue;
            }

            info!("Sending review reminder for PR #{}", discussion.pr_id);

            let mut message = CreateMessage::new();
            if let Some(review_ping_role) = config.get_review_ping_role().await {
                message = message
                    .allowed_mentions(CreateAllowedMentions::new().roles([review_ping_role]))
                    .content(format!("<@&{}>", review_ping_role.get()));
            }
            let passed = discussion
                .review_days_passed
                .expect("If micros were set, this should be too");
            let total = discussion
                .review_days_total
                .expect("If micros were set, this should be too");

            message = message
                .add_embed(
                    CreateEmbed::new()
                        .title("Review reminder")
                        .description(format!(
                            "Day {} of {} has passed. {}",
                            passed,
                            total,
                            if passed == total {
                                "It's time to come to a decision."
                            } else if passed > total {
                                "This review is overdue."
                            } else {
                                ""
                            }
                        )),
                )
                .button(
                    CreateButton::new(format!(
                        "{INTERACTION_ID_PREFIX}_{BUTTON_ID_ACTION_MUTE_REMINDERS}_{}",
                        discussion.pr_id
                    ))
                    .label("Disable reminders"),
                );

            if let Err(e) = discussion.thread_id.send_message(&ctx, message).await {
                error!("Failed to send review timer notification for {discussion:?}: {e}")
            }

            let issue_count = discussion
                .count_raised_issues(&db)
                .await
                .and_then(|x| Ok(x.to_string()))
                .unwrap_or("ERROR".into());

            let override_count = discussion
                .count_overrides(&db)
                .await
                .and_then(|x| Ok(x.to_string()))
                .unwrap_or("ERROR".into());

            let message = CreateMessage::new()
                .embed(CreateEmbed::new().title("Issues").description(format!(
                "There are currently {issue_count} issues and {override_count} votes to override."
            ))).components(
                vec![
                    CreateActionRow::Buttons(vec![
                        CreateButton::new(format!("{INTERACTION_ID_PREFIX}_{BUTTON_ID_ACTION_VIEW_ISSUES}_{}", discussion.pr_id)).label("View")
                    ])
                ]);

            if let Err(e) = discussion.thread_id.send_message(&ctx, message).await {
                error!("Failed to send issue summary for {discussion:?}: {e}")
            }
        }

        sleep(Duration::from_secs(10)).await
    }
}
