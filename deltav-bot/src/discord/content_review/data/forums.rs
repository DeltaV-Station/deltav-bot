use poise::serenity_prelude::{ChannelId, ForumTagId};
use sqlx::{Pool, Sqlite};
use tracing::{error, info, warn};

use crate::discord::{
    HandledError,
    content_review::data::{config::CrConfig, macros::id_to_int},
};

#[derive(Debug, Copy, Clone)]
pub struct ForumRecord {
    pub channel_id: ChannelId,
    pub private: bool,
    pub tag_cr_approved: ForumTagId,
    pub tag_cr_denied: ForumTagId,
    pub tag_cr_test_merge: ForumTagId,
    pub tag_pr_merged: ForumTagId,
    pub tag_pr_closed: ForumTagId,
}

impl ForumRecord {
    pub async fn get_by_channel(db: &Pool<Sqlite>, forum: ChannelId) -> Option<ForumRecord> {
        id_to_int!(forum);
        match sqlx::query!("SELECT * FROM cr_forums WHERE channel_id = ?1", forum)
            .fetch_one(db)
            .await
        {
            Ok(r) => Some(ForumRecord {
                channel_id: ChannelId::new(r.channel_id.cast_unsigned()),
                private: r.private == 1,
                tag_cr_approved: ForumTagId::new(r.tag_cr_approved.cast_unsigned()),
                tag_cr_denied: ForumTagId::new(r.tag_cr_denied.cast_unsigned()),
                tag_cr_test_merge: ForumTagId::new(r.tag_cr_test_merge.cast_unsigned()),
                tag_pr_closed: ForumTagId::new(r.tag_pr_closed.cast_unsigned()),
                tag_pr_merged: ForumTagId::new(r.tag_pr_merged.cast_unsigned()),
            }),
            Err(e) => {
                error!("Failed to get_forum {forum}: {e}");
                None
            }
        }
    }

    pub async fn upsert(&self, db: &Pool<Sqlite>) -> Result<(), HandledError> {
        let private: i64 = if self.private { 1 } else { 0 };
        let channel_id_s = self.channel_id.get().cast_signed();
        let tag_approved_s = self.tag_cr_approved.get().cast_signed();
        let tag_denied_s = self.tag_cr_denied.get().cast_signed();
        let tag_test_merge_s = self.tag_cr_test_merge.get().cast_signed();
        let tag_closed_s = self.tag_pr_closed.get().cast_signed();
        let tag_merged_s = self.tag_pr_merged.get().cast_signed();

        info!("Trying to upsert direction forum: {self:?}",);
        match sqlx::query!(
        r#"
        INSERT INTO cr_forums (channel_id, private, tag_cr_approved, tag_cr_denied, tag_cr_test_merge, tag_pr_closed, tag_pr_merged)
        VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(channel_id)
        DO UPDATE SET private=excluded.private, tag_cr_approved=excluded.tag_cr_approved, tag_cr_denied=excluded.tag_cr_denied,
        tag_cr_test_merge=excluded.tag_cr_test_merge, tag_pr_closed=excluded.tag_pr_closed, tag_pr_merged=excluded.tag_pr_merged;
        "#,
        channel_id_s, private, tag_approved_s, tag_denied_s, tag_test_merge_s, tag_closed_s, tag_merged_s
    )
    .execute(db)
    .await
    {
        Ok(_) => {
            info!("Successfully upserted direction forum {}.", self.channel_id);
            return Ok(());
        }
        Err(e) => {
            error!("Failed to upsert direction forum: {e}");
            return Err(HandledError::InternalError);
        }
    };
    }

    pub async fn delete(&self, db: &Pool<Sqlite>, config: &CrConfig) -> Result<(), HandledError> {
        delete_forum_by_channel(db, config, self.channel_id).await
    }
}

pub async fn delete_forum_by_channel(
    db: &Pool<Sqlite>,
    config: &CrConfig,
    channel_id: ChannelId,
) -> Result<(), HandledError> {
    if config.get_intake_forum().await == Some(channel_id) {
        warn!("Intake CR forum is being deleted.");
        config.set_intake_forum(None).await?;
    }

    if config.get_private_forum().await == Some(channel_id) {
        warn!("Private CR forum is being deleted.");
        config.set_private_forum(None).await?;
    }

    if config.get_public_forum().await == Some(channel_id) {
        warn!("Public CR forum is being deleted.");
        config.set_public_forum(None).await?;
    }

    id_to_int!(channel_id);

    match sqlx::query!("DELETE FROM cr_forums WHERE channel_id = ?1", channel_id)
        .execute(db)
        .await
    {
        Ok(r) => {
            if r.rows_affected() != 0 {
                info!("Deleted direction forum {}.", channel_id);
            }
            return Ok(());
        }
        Err(e) => {
            error!("Failed to delete direction forum {channel_id}: {e}");
            return Err(HandledError::InternalError);
        }
    };
}
