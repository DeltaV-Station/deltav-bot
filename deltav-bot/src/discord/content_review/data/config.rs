use poise::serenity_prelude::{ChannelId, RoleId};
use sqlx::{Pool, Sqlite};
use tracing::{error, info, warn};

use crate::discord::content_review::HandledError;

// TODO: This should hold a cache and be passed around
pub struct Config {}

impl Config {
    pub async fn get_intake_forum(db: &Pool<Sqlite>) -> Option<ChannelId> {
        let row = match sqlx::query!("SELECT intake_cr_forum FROM cr_config WHERE id = 1")
            .fetch_optional(db)
            .await
        {
            Ok(Some(x)) => x,
            Ok(None) => {
                warn!("Missing config row.");
                return None;
            }
            Err(e) => {
                error!("Failed to fetch intake CR forum: {e}");
                return None;
            }
        };

        row.intake_cr_forum
            .and_then(|x| Some(ChannelId::new(x.cast_unsigned())))
    }

    pub async fn set_intake_forum(
        db: &Pool<Sqlite>,
        channel_id: Option<ChannelId>,
    ) -> Result<(), HandledError> {
        let new_id = channel_id.and_then(|x| Some(x.get().cast_signed()));
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, intake_cr_forum)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET intake_cr_forum=excluded.intake_cr_forum;
            "#,
            new_id
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                info!("Intake CR forum set to {channel_id:?}.");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set intake CR forum: {e}");
                return Err(HandledError::UserfacingError(
                    "Failed to set intake forum. Did you register it as a forum first?".into(),
                ));
            }
        };
    }

    pub async fn get_public_forum(db: &Pool<Sqlite>) -> Option<ChannelId> {
        let row = match sqlx::query!("SELECT public_cr_forum FROM cr_config WHERE id = 1")
            .fetch_optional(db)
            .await
        {
            Ok(Some(x)) => x,
            Ok(None) => {
                warn!("Missing config row.");
                return None;
            }
            Err(e) => {
                error!("Failed to fetch public CR forum: {e}");
                return None;
            }
        };

        row.public_cr_forum
            .and_then(|x| Some(ChannelId::new(x.cast_unsigned())))
    }

    pub async fn set_public_forum(
        db: &Pool<Sqlite>,
        channel_id: Option<ChannelId>,
    ) -> Result<(), HandledError> {
        let new_id = channel_id.and_then(|x| Some(x.get().cast_signed()));
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, public_cr_forum)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET public_cr_forum=excluded.public_cr_forum;
            "#,
            new_id
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                info!("Public CR forum set to {channel_id:?}.");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set public CR forum: {e}");
                return Err(HandledError::UserfacingError(
                    "Did you register it as a forum first?".into(),
                ));
            }
        };
    }

    pub async fn get_private_forum(db: &Pool<Sqlite>) -> Option<ChannelId> {
        let row = match sqlx::query!("SELECT private_cr_forum FROM cr_config WHERE id = 1")
            .fetch_optional(db)
            .await
        {
            Ok(Some(x)) => x,
            Ok(None) => {
                warn!("Missing config row.");
                return None;
            }
            Err(e) => {
                error!("Failed to fetch private CR forum: {e}");
                return None;
            }
        };

        row.private_cr_forum
            .and_then(|x| Some(ChannelId::new(x.cast_unsigned())))
    }

    pub async fn set_private_forum(
        db: &Pool<Sqlite>,
        channel_id: Option<ChannelId>,
    ) -> Result<(), HandledError> {
        let new_id = channel_id.and_then(|x| Some(x.get().cast_signed()));
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, private_cr_forum)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET private_cr_forum=excluded.private_cr_forum;
            "#,
            new_id
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                info!("Private CR forum set to {channel_id:?}.");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set private CR forum: {e}");
                return Err(HandledError::UserfacingError(
                    "Did you register it as a forum first?".into(),
                ));
            }
        };
    }

    pub async fn set_no_review_needed_label(
        db: &Pool<Sqlite>,
        label: String,
    ) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, gh_label_no_review)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET gh_label_no_review=excluded.gh_label_no_review;
            "#,
            label
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                info!("No review needed label set to '{label}'.");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set no review needed label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_under_review_label(db: &Pool<Sqlite>) -> Option<String> {
        let row = match sqlx::query!(
            r#"
            SELECT gh_label_under_review
            FROM cr_config
            WHERE ID = 1
            "#,
        )
        .fetch_optional(db)
        .await
        {
            Ok(Some(x)) => x,
            Ok(None) => {
                warn!("Missing config row.");
                return None;
            }
            Err(e) => {
                error!("Failed to fetch under review needed label: {e}");
                return None;
            }
        };

        row.gh_label_under_review
    }

    pub async fn set_under_review_label(
        db: &Pool<Sqlite>,
        label: String,
    ) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, gh_label_under_review)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET gh_label_under_review=excluded.gh_label_under_review;
            "#,
            label
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                info!("Under review label set to '{label}'.");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set under review label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_no_review_needed_label(db: &Pool<Sqlite>) -> Option<String> {
        let row = match sqlx::query!(
            r#"
            SELECT gh_label_no_review
            FROM cr_config
            WHERE ID = 1
            "#,
        )
        .fetch_optional(db)
        .await
        {
            Ok(Some(x)) => x,
            Ok(None) => {
                warn!("Missing config row.");
                return None;
            }
            Err(e) => {
                error!("Failed to fetch no review needed label: {e}");
                return None;
            }
        };

        row.gh_label_no_review
    }

    pub async fn get_approved_label(db: &Pool<Sqlite>) -> Option<String> {
        let row = match sqlx::query!(
            r#"
            SELECT gh_label_cr_approved
            FROM cr_config
            WHERE ID = 1
            "#,
        )
        .fetch_optional(db)
        .await
        {
            Ok(Some(x)) => x,
            Ok(None) => {
                warn!("Missing config row.");
                return None;
            }
            Err(e) => {
                error!("Failed to fetch approved label: {e}");
                return None;
            }
        };

        row.gh_label_cr_approved
    }

    pub async fn set_approved_label(db: &Pool<Sqlite>, label: String) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, gh_label_cr_approved)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET gh_label_cr_approved=excluded.gh_label_cr_approved;
            "#,
            label
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                info!("CR approved label set to '{label}'.");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set approved label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_denied_label(db: &Pool<Sqlite>) -> Option<String> {
        let row = match sqlx::query!(
            r#"
            SELECT gh_label_cr_denied
            FROM cr_config
            WHERE ID = 1
            "#,
        )
        .fetch_optional(db)
        .await
        {
            Ok(Some(x)) => x,
            Ok(None) => {
                warn!("Missing config row.");
                return None;
            }
            Err(e) => {
                error!("Failed to fetch CR denied label: {e}");
                return None;
            }
        };

        row.gh_label_cr_denied
    }

    pub async fn set_denied_label(db: &Pool<Sqlite>, label: String) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, gh_label_cr_denied)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET gh_label_cr_denied=excluded.gh_label_cr_denied;
            "#,
            label
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                info!("CR Denied label set to '{label}'.");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set CR denied label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_review_ping_role(db: &Pool<Sqlite>) -> Option<RoleId> {
        let row = match sqlx::query!("SELECT review_ping_role FROM cr_config WHERE id = 1")
            .fetch_optional(db)
            .await
        {
            Ok(Some(x)) => x,
            Ok(None) => {
                warn!("Missing config row.");
                return None;
            }
            Err(e) => {
                error!("Failed to review ping role: {e}");
                return None;
            }
        };

        row.review_ping_role
            .and_then(|x| Some(RoleId::new(x.cast_unsigned())))
    }

    pub async fn set_review_ping_role(
        db: &Pool<Sqlite>,
        role_id: Option<RoleId>,
    ) -> Result<(), HandledError> {
        let new_id = role_id.and_then(|x| Some(x.get().cast_signed()));
        match sqlx::query!(
            r#"
                INSERT INTO cr_config (id, review_ping_role)
                VALUES(1, ?1)
                ON CONFLICT(id)
                DO UPDATE SET review_ping_role=excluded.review_ping_role;
                "#,
            new_id
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                info!("Review ping role set to {role_id:?}.");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set review ping role: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }
}
