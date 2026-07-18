use octocrab::params::pulls;
use poise::{
    ChoiceParameter, CreateReply, Modal,
    serenity_prelude::{CreateEmbed, CreateEmbedFooter, EditThread},
};
use tracing::error;

use crate::discord::{
    ApplicationContext, Context, Error,
    content_review::{util::get_channel_discussion, util::try_remove_label},
    permissions::{check_permissions_command, data::PermissionFlags},
    to_md_quote_block,
};

#[derive(ChoiceParameter)]
pub enum CrOutcome {
    #[name = "Test-Merge"]
    TestMerge,
    #[name = "Approved"]
    Approved,
    #[name = "Denied"]
    Denied,
}

#[derive(Debug, Modal)]
#[name = "Request changes"]
struct RequestChangesModal {
    #[name = "What should be changed?"]
    #[placeholder = "If you prefer writing the comment yourself, leave this field blank to only add the label."]
    #[paragraph]
    description: String,
}

/// Request changes from the author. This command will open a pop-up with a text field for your request.
#[poise::command(slash_command, rename = "request-changes", ephemeral)]
pub async fn cr_request_changes(ctx: ApplicationContext<'_>) -> Result<(), Error> {
    let wrapped_ctx = Context::Application(ctx);
    if !check_permissions_command(&wrapped_ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    let (_, mut discussion, _) = match get_channel_discussion(&wrapped_ctx).await {
        Ok((forum, discussion, channel)) => (forum, discussion, channel),
        Err(e) => {
            ctx.reply(format!("Error: {e}")).await?;
            return Ok(());
        }
    };

    let Some(under_review_label) = ctx.data().cr_config.get_under_review_label().await else {
        ctx.reply("Can't process change request with under review label unset.")
            .await?;
        return Ok(());
    };

    let Some(changes_requested_label) = ctx.data().cr_config.get_changes_requested_label().await
    else {
        ctx.reply("Can't process change request with changes requested label unset.")
            .await?;
        return Ok(());
    };

    let Some(response) = RequestChangesModal::execute(ctx).await? else {
        ctx.reply("Modal timed out.").await?;
        return Ok(());
    };

    let issues = ctx.data().gh.octo_install.issues_by_id(ctx.data().gh.repo);

    if !response.description.is_empty() {
        if let Err(e) = issues
            .create_comment(
                discussion.pr_id,
                format!(
                    "**Changes requested by CR**\n{}Sent by {}.",
                    to_md_quote_block(&response.description),
                    ctx.author().name
                ),
            )
            .await
        {
            error!(
                "Failed to create change request comment on PR #{}: {e:#?}",
                discussion.pr_id
            );

            ctx.reply("Failed to create GitHub comment").await?;
            return Ok(());
        }
    }

    ctx.send(CreateReply::default()
        .ephemeral(false)
        .embed(CreateEmbed::new()
            .title("Changes requested")
            .description(
                if response.description.is_empty() {
                    "Description was unset. Label has been applied, please write a comment describing the required changes yourself.".into()
                } else
                {
                    response.description
                })
            .footer(CreateEmbedFooter::new(&ctx.author().name))
        )
    ).await?;

    if let Err(e) = issues
        .add_labels(discussion.pr_id, &[changes_requested_label])
        .await
    {
        error!(
            "Failed to set changes requested label on PR #{}: {e:#?}",
            discussion.pr_id
        );

        ctx.reply("Failed to set changes requested label").await?;
        return Ok(());
    }

    try_remove_label(
        &ctx.data().gh,
        &under_review_label,
        &Context::Application(ctx),
        &discussion,
    )
    .await?;

    if let Err(e) = discussion.disable_reminders(&ctx.data().db).await {
        ctx.reply(format!(
            "Failed to disable reminders upon change request: {e}"
        ))
        .await?;
    }

    Ok(())
}

/// Determine the outcome of the review, apply labels and notify the author.
#[poise::command(slash_command, rename = "complete", ephemeral)]
pub async fn cr_complete(
    ctx: Context<'_>,
    #[description = "The outcome of the review"] outcome: CrOutcome,
    #[description = "Anything you want the author to know?"] comment: Option<String>,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::CONTENT_REVIEWER).await? {
        return Ok(());
    }

    let (forum, mut discussion, mut channel) = match get_channel_discussion(&ctx).await {
        Ok((forum, discussion, channel)) => (forum, discussion, channel),
        Err(e) => {
            ctx.reply(format!("Error: {e}")).await?;
            return Ok(());
        }
    };

    let Some(under_review_label) = ctx.data().cr_config.get_under_review_label().await else {
        ctx.reply("Can't close with unset Under Review label.")
            .await?;
        return Ok(());
    };

    let gh = &ctx.data().gh;
    let config = &ctx.data().cr_config;
    let issues = gh.octo_install.issues_by_id(gh.repo);
    match outcome {
        CrOutcome::Approved | CrOutcome::TestMerge => {
            let Some(approved_label) = config.get_approved_label().await else {
                ctx.reply("Can't close with unset CR Approved label.")
                    .await?;
                return Ok(());
            };

            ctx.defer().await?;

            if let Err(e) = issues.add_labels(discussion.pr_id, &[approved_label]).await {
                error!(
                    "Failed to set CR Approved label on PR #{}: {e}",
                    discussion.pr_id
                );

                ctx.reply("Failed to add CR Approved GitHub label.").await?;
                return Ok(());
            };
        }
        CrOutcome::Denied => {
            let Some(denied_label) = config.get_denied_label().await else {
                ctx.reply("Can't close with unset CR Denied label.").await?;
                return Ok(());
            };

            ctx.defer().await?;

            if let Err(e) = issues.add_labels(discussion.pr_id, &[denied_label]).await {
                error!(
                    "Failed to set CR Denied label on PR #{}: {e}",
                    discussion.pr_id
                );

                ctx.reply("Failed to add CR Denied GitHub label.").await?;
                return Ok(());
            };

            if let Err(e) = gh
                .octo_install
                .pulls(&gh.repo_owner, &gh.repo_name)
                .update(discussion.pr_id)
                .state(pulls::State::Closed)
                .send()
                .await
            {
                error!("Failed to close PR #{}: {e}", discussion.pr_id);

                ctx.reply("Failed to close PR.").await?;
                // Not returning here since closing is really not essential, it's already marked
            }
        }
    }

    if let Err(e) = discussion.disable_reminders(&ctx.data().db).await {
        ctx.reply(format!("Failed to disable reminders during closing: {e}"))
            .await?;
    }

    try_remove_label(&ctx.data().gh, &under_review_label, &ctx, &discussion).await?;
    if let Some(changes_requested_label) = config.get_changes_requested_label().await {
        try_remove_label(&ctx.data().gh, changes_requested_label, &ctx, &discussion).await?;
    }

    if let Err(e) = issues
        .create_comment(
            discussion.pr_id,
            format!(
                "**CR consensus: {}**\n{}Review closed by {}.",
                outcome.name(),
                if let Some(comment) = &comment {
                    to_md_quote_block(comment)
                } else {
                    String::new()
                },
                ctx.author().name
            ),
        )
        .await
    {
        error!(
            "Failed to create CR outcome comment on PR #{}: {e}",
            discussion.pr_id
        );

        ctx.reply("Failed to create GitHub comment.").await?;
        return Ok(());
    };

    ctx.send(
        CreateReply::default().embed(
            CreateEmbed::new()
                .title(format!("Discussion closed: {}", outcome.name()))
                .description(comment.unwrap_or("*No reasoning provided.*".into()))
                .footer(CreateEmbedFooter::new(format!(
                    "Closed by {}",
                    ctx.author().name
                ))),
        ),
    )
    .await?;

    let tag = match outcome {
        CrOutcome::TestMerge => forum.tag_cr_test_merge,
        CrOutcome::Approved => forum.tag_cr_approved,
        CrOutcome::Denied => forum.tag_cr_denied,
    };

    if let Err(e) = channel
        .edit_thread(
            &ctx,
            EditThread::new()
                .applied_tags(channel.applied_tags.iter().chain([tag].iter()).cloned()),
        )
        .await
    {
        error!(
            "Failed to label thread for PR#{} ({}): {e}",
            discussion.pr_id, channel.id
        );
        ctx.reply("Failed to label thread. Lacking permissions?")
            .await?;
    }

    if let Err(e) = channel
        .edit_thread(&ctx, EditThread::new().archived(true))
        .await
    {
        error!(
            "Failed to archive thread for PR#{} ({}): {e}",
            discussion.pr_id, channel.id
        );
        ctx.reply("Failed to archive thread. Lacking permissions?")
            .await?;
    }

    Ok(())
}
