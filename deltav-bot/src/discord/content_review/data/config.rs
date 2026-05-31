use std::sync::Arc;

use poise::serenity_prelude::{ChannelId, RoleId};
use sqlx::{Pool, Sqlite};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::discord::content_review::HandledError;

#[derive(Clone)]
pub struct CrConfig {
    db: Pool<Sqlite>,
    cache: Arc<RwLock<ConfigCache>>,
}

#[derive(Default)]
struct ConfigCache {
    intake_forum: Option<ChannelId>,
    private_forum: Option<ChannelId>,
    public_forum: Option<ChannelId>,

    ghl_under_review: Option<String>,
    ghl_not_needed: Option<String>,
    ghl_approved: Option<String>,
    ghl_denied: Option<String>,
    ghl_changes_requested: Option<String>,

    review_ping_role: Option<RoleId>,
}

impl CrConfig {
    pub async fn from_db(db: Pool<Sqlite>) -> Result<Self, Box<dyn std::error::Error>> {
        info!("Loading CR Config from DB.");
        match sqlx::query!("SELECT * FROM cr_config WHERE id = 1")
            .fetch_optional(&db)
            .await
        {
            Ok(Some(x)) => Ok(Self {
                db,
                cache: Arc::new(RwLock::new(ConfigCache {
                    ghl_approved: x.gh_label_cr_approved,
                    ghl_denied: x.gh_label_cr_denied,
                    ghl_not_needed: x.gh_label_no_review,
                    ghl_under_review: x.gh_label_under_review,
                    ghl_changes_requested: x.gh_label_cr_changes_requested,
                    intake_forum: x
                        .intake_cr_forum
                        .and_then(|x| Some(ChannelId::new(x.cast_unsigned()))),
                    private_forum: x
                        .private_cr_forum
                        .and_then(|x| Some(ChannelId::new(x.cast_unsigned()))),
                    public_forum: x
                        .public_cr_forum
                        .and_then(|x| Some(ChannelId::new(x.cast_unsigned()))),
                    review_ping_role: x
                        .review_ping_role
                        .and_then(|x| Some(RoleId::new(x.cast_unsigned()))),
                })),
            }),
            Ok(None) => {
                warn!("Missing config row.");

                Ok(Self {
                    db,
                    cache: Arc::new(RwLock::new(ConfigCache::default())),
                })
            }
            Err(e) => {
                error!("Failed to fetch config row: {e}");
                Err(Box::new(e))
            }
        }
    }

    pub async fn get_intake_forum(&self) -> Option<ChannelId> {
        self.cache.read().await.intake_forum
    }

    pub async fn set_intake_forum(
        &self,
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
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("Intake CR forum set to {channel_id:?}.");
                self.cache.write().await.intake_forum = channel_id;
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

    pub async fn get_public_forum(&self) -> Option<ChannelId> {
        self.cache.read().await.public_forum
    }

    pub async fn set_public_forum(
        &self,
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
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("Public CR forum set to {channel_id:?}.");
                self.cache.write().await.public_forum = channel_id;
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

    pub async fn get_private_forum(&self) -> Option<ChannelId> {
        self.cache.read().await.private_forum
    }

    pub async fn set_private_forum(
        &self,
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
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("Private CR forum set to {channel_id:?}.");
                self.cache.write().await.private_forum = channel_id;
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

    pub async fn set_no_review_needed_label(&self, label: String) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, gh_label_no_review)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET gh_label_no_review=excluded.gh_label_no_review;
            "#,
            label
        )
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("No review needed label set to '{label}'.");
                self.cache.write().await.ghl_not_needed = Some(label);
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set no review needed label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_no_review_needed_label(&self) -> Option<String> {
        self.cache.read().await.ghl_not_needed.clone()
    }

    pub async fn get_under_review_label(&self) -> Option<String> {
        self.cache.read().await.ghl_under_review.clone()
    }

    pub async fn set_under_review_label(&self, label: String) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, gh_label_under_review)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET gh_label_under_review=excluded.gh_label_under_review;
            "#,
            label
        )
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("Under review label set to '{label}'.");
                self.cache.write().await.ghl_under_review = Some(label);
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set under review label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_changes_requested_label(&self) -> Option<String> {
        self.cache.read().await.ghl_changes_requested.clone()
    }

    pub async fn set_changes_requested_label(&self, label: String) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
                INSERT INTO cr_config (id, gh_label_cr_changes_requested)
                VALUES(1, ?1)
                ON CONFLICT(id)
                DO UPDATE SET gh_label_cr_changes_requested=excluded.gh_label_cr_changes_requested;
                "#,
            label
        )
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("Changes requested label set to '{label}'.");
                self.cache.write().await.ghl_changes_requested = Some(label);
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set changes requested label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_approved_label(&self) -> Option<String> {
        self.cache.read().await.ghl_approved.clone()
    }

    pub async fn set_approved_label(&self, label: String) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, gh_label_cr_approved)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET gh_label_cr_approved=excluded.gh_label_cr_approved;
            "#,
            label
        )
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("CR approved label set to '{label}'.");
                self.cache.write().await.ghl_approved = Some(label);
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set approved label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_denied_label(&self) -> Option<String> {
        self.cache.read().await.ghl_denied.clone()
    }

    pub async fn set_denied_label(&self, label: String) -> Result<(), HandledError> {
        match sqlx::query!(
            r#"
            INSERT INTO cr_config (id, gh_label_cr_denied)
            VALUES(1, ?1)
            ON CONFLICT(id)
            DO UPDATE SET gh_label_cr_denied=excluded.gh_label_cr_denied;
            "#,
            label
        )
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("CR Denied label set to '{label}'.");
                self.cache.write().await.ghl_denied = Some(label);
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set CR denied label: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_review_ping_role(&self) -> Option<RoleId> {
        self.cache.read().await.review_ping_role
    }

    pub async fn set_review_ping_role(&self, role_id: Option<RoleId>) -> Result<(), HandledError> {
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
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                info!("Review ping role set to {role_id:?}.");
                self.cache.write().await.review_ping_role = role_id;
                return Ok(());
            }
            Err(e) => {
                error!("Failed to set review ping role: {e}");
                return Err(HandledError::InternalError);
            }
        };
    }

    pub async fn get_cr_github_labels(&self) -> Vec<String> {
        let mut out = Vec::new();

        if let Some(approved) = self.get_approved_label().await {
            out.push(approved);
        }

        if let Some(denied) = self.get_denied_label().await {
            out.push(denied);
        }

        if let Some(under_review) = self.get_under_review_label().await {
            out.push(under_review);
        }

        if let Some(changes_requested) = self.get_changes_requested_label().await {
            out.push(changes_requested);
        }

        out
    }
}
