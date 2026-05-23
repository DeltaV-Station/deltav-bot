use poise::serenity_prelude::{RoleId, UserId};

use crate::discord::{Context, Error, permissions::data::PermissionFlags};

pub mod data;

#[poise::command(slash_command, subcommands("perms_get"))]
pub async fn perms(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

#[poise::command(slash_command, rename = "get")]
pub async fn perms_get(
    ctx: Context<'_>,
    user: Option<UserId>,
    role: Option<RoleId>,
) -> Result<(), Error> {
    if !ctx
        .data()
        .permissions
        .has_flags(ctx.author().id.get(), PermissionFlags::PERMISSIONS_VIEW)
        .await
    {
        return Ok(());
    }

    ctx.defer_ephemeral().await?;

    if user.is_some() && role.is_some() {
        ctx.reply("You may only specify one of: user, role").await?;
        return Ok(());
    }

    let mut snowflake = u64::MIN;

    if let Some(user) = user {
        snowflake = user.get();
    }

    if let Some(role) = role {
        snowflake = role.get();
    }

    if snowflake == u64::MIN {
        ctx.reply("You must specify one of: user, role").await?;
        return Ok(());
    }

    match ctx
        .data()
        .permissions
        .get_flags(ctx.author().id.get())
        .await
    {
        Ok(flags) => {
            let message = flags
                .iter_names()
                .fold(String::new(), |out, (name, _)| format!("{out}- `{name}`\n"));
            ctx.reply(message).await?;
        }
        Err(()) => {
            ctx.reply("Failed to retrieve flags.").await?;
        }
    }

    Ok(())
}

pub async fn respond_unauthorized(ctx: &Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    ctx.reply("You are not authorized to use this feature.")
        .await?;
    Ok(())
}
