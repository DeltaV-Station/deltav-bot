use std::sync::Arc;

use poise::serenity_prelude::{ChannelId, RoleId};
use sqlx::{Pool, Sqlite, query};
use tokio::sync::RwLock;
use tracing::error;

use crate::discord::content_review::HandledError;

#[derive(Clone)]
pub struct PrFeeds {
    db: Pool<Sqlite>,
    feeds: Arc<RwLock<Vec<PrFeed>>>,
}

#[derive(Clone)]
pub struct PrFeed {
    pub id: i64,
    pub gh_label: String,
    pub channel_id: ChannelId,
    pub ping_role: Option<RoleId>,
}

impl PrFeeds {
    pub async fn from_db(db: Pool<Sqlite>) -> Result<Self, sqlx::Error> {
        let feeds: Vec<PrFeed> = query!("SELECT * FROM prfeeds;")
            .fetch_all(&db)
            .await
            .and_then(|x| {
                Ok(x.iter()
                    .map(|x| PrFeed {
                        channel_id: ChannelId::new(x.channel_id.cast_unsigned()),
                        gh_label: x.gh_label.clone(),
                        id: x.id,
                        ping_role: x
                            .ping_role
                            .and_then(|y| Some(RoleId::new(y.cast_unsigned()))),
                    })
                    .collect())
            })?;

        Ok(Self {
            db,
            feeds: Arc::new(RwLock::new(feeds)),
        })
    }

    pub async fn get_all(&self) -> Vec<PrFeed> {
        self.feeds.read().await.clone()
    }

    pub async fn get(&self, id: i64) -> Option<PrFeed> {
        self.feeds.read().await.iter().find(|x| x.id == id).cloned()
    }

    pub async fn get_by_label(&self, label: impl AsRef<str>) -> Option<PrFeed> {
        self.feeds
            .read()
            .await
            .iter()
            .find(|x| x.gh_label == label.as_ref())
            .cloned()
    }

    pub async fn add(
        &self,
        gh_label: impl Into<String>,
        channel_id: ChannelId,
        ping_role: Option<RoleId>,
    ) -> Result<(), HandledError> {
        let channel_id_s = channel_id.get().cast_signed();
        let ping_role_id_s = ping_role.and_then(|x| Some(x.get().cast_signed()));
        let gh_label = gh_label.into();

        let new_feed = match query!(
            "INSERT INTO prfeeds(gh_label, channel_id, ping_role) VALUES(?1,?2,?3) RETURNING *",
            gh_label,
            channel_id_s,
            ping_role_id_s
        )
        .fetch_one(&self.db)
        .await
        {
            Err(e) => {
                error!(
                    "Failed to add new PR feed for label '{gh_label}' in channel {channel_id}: {e}"
                );
                return Err(HandledError::InternalError);
            }
            Ok(x) => PrFeed {
                id: x.id,
                channel_id: ChannelId::new(x.channel_id.cast_unsigned()),
                gh_label: x.gh_label,
                ping_role: x
                    .ping_role
                    .and_then(|x| Some(RoleId::new(x.cast_unsigned()))),
            },
        };

        self.feeds.write().await.push(new_feed);

        Ok(())
    }

    pub async fn remove(&self, id: i64) -> Result<(), HandledError> {
        if let Err(e) = query!("DELETE FROM prfeeds WHERE id = ?1", id)
            .execute(&self.db)
            .await
        {
            error!("Failed to delete PR feed {id}: {e}");
            return Err(HandledError::InternalError);
        }

        let mut feeds = self.feeds.write().await;
        let cache_index = feeds.iter().position(|x| x.id == id);

        if let Some(cache_index) = cache_index {
            feeds.remove(cache_index);
        }
        Ok(())
    }
}
