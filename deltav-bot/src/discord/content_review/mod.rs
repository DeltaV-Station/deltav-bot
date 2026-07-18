use crate::discord::{
    Context, Error,
    content_review::{data::config::commands::*, raised_issues::cr_issue, triage::commands::*},
};

pub mod component_events;
pub mod consts;
pub mod data;
pub mod github_events;
pub mod raised_issues;
pub mod timers;
pub mod triage;
pub mod util;

#[poise::command(
    slash_command,
    subcommands(
        "cr_forum",
        "cr_config",
        "cr_complete",
        "cr_request_changes",
        "cr_ignored",
        "cr_issue"
    )
)]
pub async fn cr(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}
