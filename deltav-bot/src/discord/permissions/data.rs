use std::{collections::HashMap, sync::Arc};

use bitflags::bitflags;
use sqlx::{Pool, Sqlite, query};
use tokio::sync::RwLock;
use tracing::{error, info};

use sqlx::Error as SqlxError;

use crate::discord::content_review::HandledError;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PermissionFlags: u64 {
        const CONTENT_REVIEW_CONFIG = 1;
        const CONTENT_REVIEWER      = 1 << 1;
        const PERMISSIONS_VIEW      = 1 << 2;
        const PERMISSIONS_EDIT      = 1 << 3;
    }
}

pub type Snowflake = u64;

#[derive(Clone)]
pub struct Permissions {
    db: Pool<Sqlite>,
    cache: Arc<RwLock<HashMap<Snowflake, PermissionFlags>>>,
}

impl Permissions {
    pub fn new(db: Pool<Sqlite>) -> Self {
        Self {
            db,
            cache: Arc::new(RwLock::new(HashMap::new())), // TODO: cache eviction
        }
    }

    pub async fn has_flags(&self, id: Snowflake, expected_flags: PermissionFlags) -> bool {
        match self.get_flags(id).await {
            Ok(actual_flags) => actual_flags.contains(expected_flags),
            Err(_) => false,
        }
    }

    pub async fn set_flags(
        &self,
        id: Snowflake,
        flags: PermissionFlags,
    ) -> Result<(), HandledError> {
        let id_s = id.cast_signed();
        if flags.is_empty() {
            match query!("DELETE FROM permissions WHERE snowflake = ?1", id_s)
                .execute(&self.db)
                .await
            {
                Ok(_) => {
                    self.cache.write().await.insert(id, flags);

                    return Ok(());
                }
                Err(e) => {
                    if let SqlxError::RowNotFound = e {
                        return Ok(());
                    }

                    error!("Failed to delete permissions row for {id}: {e}");
                    return Err(HandledError::InternalError);
                }
            }
        }

        let flags_s = flags.bits().cast_signed();
        match query!(
            "INSERT INTO permissions(snowflake, flags) VALUES(?1, ?2) ON CONFLICT(snowflake) DO UPDATE SET flags=excluded.flags",
            id_s,
            flags_s,
        )
        .execute(&self.db)
        .await
        {
            Ok(_) => {
                self.cache.write().await.insert(id, flags);
                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to set permission flags for {id} to {:b}: {e}",
                    flags.bits()
                );

                Err(HandledError::InternalError)
            }
        }
    }

    pub async fn get_flags(&self, id: Snowflake) -> Result<PermissionFlags, HandledError> {
        if let Some(cached) = self.cache.read().await.get(&id) {
            return Ok(*cached);
        }

        let id_s = id.cast_signed();
        match query!("SELECT flags FROM permissions WHERE snowflake = ?1", id_s)
            .fetch_optional(&self.db)
            .await
        {
            Ok(Some(x)) => {
                let Some(flags) = PermissionFlags::from_bits(x.flags.cast_unsigned()) else {
                    error!("Invalid permission flags for {id}: {:b}", x.flags);
                    return Err(HandledError::InternalError);
                };

                self.cache.write().await.insert(id, flags);

                Ok(flags)
            }
            Ok(None) => {
                self.cache
                    .write()
                    .await
                    .insert(id, PermissionFlags::empty());
                Ok(PermissionFlags::empty())
            }
            Err(e) => {
                error!("Unable to fetch permissions for {id}: {e}");

                Err(HandledError::InternalError)
            }
        }
    }

    pub async fn remove_flags(
        &self,
        id: Snowflake,
        flags: PermissionFlags,
    ) -> Result<(), HandledError> {
        info!("Trying to remove flags {flags:b} from {id}.");

        let new_flags = self.get_flags(id).await.map(|x| x.difference(flags))?;
        self.set_flags(id, new_flags).await?;

        Ok(())
    }

    pub async fn add_flags(
        &self,
        id: Snowflake,
        flags: PermissionFlags,
    ) -> Result<(), HandledError> {
        info!("Trying to add flags {flags:b} to {id}.");

        let new_flags = self.get_flags(id).await.map(|x| x.union(flags))?;
        self.set_flags(id, new_flags).await?;

        Ok(())
    }
}
