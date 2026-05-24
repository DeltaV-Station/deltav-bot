use std::collections::HashMap;

use bitflags::Flags;
use poise::{
    ChoiceParameter, CommandParameterChoice,
    serenity_prelude::{ComponentInteraction, CreateInteractionResponseMessage, RoleId, UserId},
};
use tracing::{error, info, warn};

use crate::discord::{
    Context, Error,
    permissions::data::{PermissionFlags, Permissions, Snowflake},
};

pub mod data;

impl ChoiceParameter for PermissionFlags {
    fn list() -> Vec<poise::CommandParameterChoice> {
        PermissionFlags::FLAGS
            .iter()
            .map(|x| CommandParameterChoice {
                name: x.name().into(),
                localizations: HashMap::new(),
                __non_exhaustive: (),
            })
            .collect()
    }

    fn from_index(index: usize) -> Option<Self> {
        PermissionFlags::FLAGS
            .get(index)
            .and_then(|x| Some(*x.value()))
    }

    fn from_name(name: &str) -> Option<Self> {
        PermissionFlags::from_name(name)
    }

    fn name(&self) -> &'static str {
        self.iter_names().next().unwrap().0
    }

    fn localized_name(&self, _locale: &str) -> Option<&'static str> {
        None
    }
}

#[poise::command(
    slash_command,
    subcommands("perms_get", "perms_add", "perms_remove", "perms_breakdown")
)]
pub async fn perms(_ctx: Context<'_>) -> Result<(), Error> {
    // dummy command
    Ok(())
}

#[poise::command(slash_command, rename = "get", ephemeral)]
pub async fn perms_get(
    ctx: Context<'_>,
    user: Option<UserId>,
    role: Option<RoleId>,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::PERMISSIONS_VIEW).await? {
        return Ok(());
    }

    let Some(snowflake) = check_id_args(&ctx, role, user).await? else {
        return Ok(());
    };

    match ctx.data().permissions.get_flags(snowflake).await {
        Ok(flags) => {
            let message = flags
                .iter_names()
                .fold(String::new(), |out, (name, _)| format!("{out}- `{name}`\n"));
            ctx.reply(if message.is_empty() {
                "No permissions granted.".into()
            } else {
                message
            })
            .await?;
        }
        Err(e) => {
            ctx.reply(format!("Failed to retrieve flags: {e}")).await?;
        }
    }

    Ok(())
}

#[poise::command(slash_command, rename = "add", ephemeral)]
pub async fn perms_add(
    ctx: Context<'_>,
    user: Option<UserId>,
    role: Option<RoleId>,
    permission: PermissionFlags,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::PERMISSIONS_EDIT).await? {
        return Ok(());
    }

    let Some(snowflake) = check_id_args(&ctx, role, user).await? else {
        return Ok(());
    };

    match ctx
        .data()
        .permissions
        .add_flags(snowflake, permission)
        .await
    {
        Ok(()) => {
            info!(
                "User {} ({}) added {} to {snowflake}",
                ctx.author().name,
                ctx.author().id,
                permission.name()
            );
            ctx.reply(format!("Added `{}` flag.", permission.name()))
                .await?;
        }
        Err(e) => {
            ctx.reply(format!("Failed to add flag: {e}")).await?;
        }
    }

    Ok(())
}

#[poise::command(slash_command, rename = "remove", ephemeral)]
pub async fn perms_remove(
    ctx: Context<'_>,
    user: Option<UserId>,
    role: Option<RoleId>,
    permission: PermissionFlags,
) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::PERMISSIONS_EDIT).await? {
        return Ok(());
    }

    let Some(snowflake) = check_id_args(&ctx, role, user).await? else {
        return Ok(());
    };

    match ctx
        .data()
        .permissions
        .remove_flags(snowflake, permission)
        .await
    {
        Ok(()) => {
            info!(
                "User {} ({}) removed {} from {snowflake}",
                ctx.author().name,
                ctx.author().id,
                permission.name()
            );

            ctx.reply(format!("Removed `{}` flag.", permission.name()))
                .await?;
        }
        Err(e) => {
            ctx.reply(format!("Failed to add flag: {e}")).await?;
        }
    }

    Ok(())
}

#[poise::command(slash_command, rename = "breakdown", ephemeral)]
pub async fn perms_breakdown(ctx: Context<'_>, user: UserId) -> Result<(), Error> {
    if !check_permissions_command(&ctx, PermissionFlags::PERMISSIONS_VIEW).await? {
        return Ok(());
    }

    let mut response = format!("## Permissions breakdown for <@{user}>\n");
    let mut any_permissions = false;

    if let Ok(user_perms) = ctx.data().permissions.get_flags(user.get()).await
        && !user_perms.is_empty()
    {
        response += &format!("### User\n");
        for (name, _) in user_perms.iter_names() {
            response += &format!("- `{name}`\n");
        }
        any_permissions = true;
    }

    if let Some(member) = ctx.author_member().await {
        for role in &member.roles {
            let role_perms = match ctx.data().permissions.get_flags(role.get()).await {
                Ok(role_perms) => role_perms,
                Err(e) => {
                    response += &format!(
                        "Encountered an error while checking permissions of role <@&{role}>: {e}"
                    );
                    ctx.reply(response).await?;
                    return Ok(());
                }
            };

            if role_perms.is_empty() {
                continue;
            }

            any_permissions = true;

            response += &format!("### Role <@&{role}>\n");
            for (name, _) in role_perms.iter_names() {
                response += &format!("- `{name}`\n");
            }
        }
    }

    ctx.reply(if !any_permissions {
        "No permissions granted.".into()
    } else {
        response
    })
    .await?;

    Ok(())
}
async fn check_id_args(
    ctx: &Context<'_>,
    role: Option<RoleId>,
    user: Option<UserId>,
) -> Result<Option<Snowflake>, Error> {
    if user.is_some() && role.is_some() {
        ctx.reply("You may only specify one of: user, role").await?;
        return Ok(None);
    }

    let mut snowflake = u64::MIN;

    if let Some(user) = user {
        snowflake = user.get();
    }

    if let Some(role) = role {
        snowflake = role.get();
    }

    if snowflake == u64::MIN {
        ctx.reply("You must specify exactly one of: user, role")
            .await?;
        return Ok(None);
    }

    Ok(Some(snowflake))
}

pub async fn check_permissions_command(
    ctx: &Context<'_>,
    flags: PermissionFlags,
) -> Result<bool, Error> {
    if ctx
        .data()
        .permissions
        .has_flags(ctx.author().id.get(), flags)
        .await
    {
        return Ok(true);
    }

    if let Some(member) = ctx.author_member().await {
        for role in &member.roles {
            if ctx.data().permissions.has_flags(role.get(), flags).await {
                return Ok(true);
            }
        }
    }

    ctx.reply("You are not authorized to use this feature.")
        .await?;
    Ok(false)
}

pub async fn check_permissions_component(
    ctx: &poise::serenity_prelude::Context,
    interaction: &ComponentInteraction,
    permissions: &Permissions,
    flags: PermissionFlags,
) -> Result<bool, Error> {
    if permissions
        .has_flags(interaction.user.id.get(), flags)
        .await
    {
        return Ok(true);
    }

    let Some(guild) = interaction.guild_id else {
        error!("Interaction without guild_id: {interaction:?}. Returning unauthorized.");
        return Ok(false);
    };

    let member = match guild.member(&ctx, interaction.user.id).await {
        Ok(member) => member,
        Err(e) => {
            error!("Couldn't get guild member for interaction: {interaction:?}. Error: {e}");
            interaction
                .create_response(
                    &ctx,
                    poise::serenity_prelude::CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .ephemeral(true)
                            .content("An internal error occurred."),
                    ),
                )
                .await?;
            return Err(Box::new(e));
        }
    };

    for role in &member.roles {
        if permissions.has_flags(role.get(), flags).await {
            return Ok(true);
        }
    }

    interaction
        .create_response(
            &ctx,
            poise::serenity_prelude::CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .ephemeral(true)
                    .content("You are not authorized to use this feature."),
            ),
        )
        .await?;

    Ok(false)
}
