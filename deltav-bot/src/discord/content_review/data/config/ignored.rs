use std::sync::Arc;

use poise::ChoiceParameter;
use sqlx::{Pool, Sqlite, query};
use strum::FromRepr;
use tokio::sync::RwLock;
use tracing::error;

use crate::discord::content_review::HandledError;

#[repr(i64)]
#[derive(FromRepr, Debug, PartialEq, ChoiceParameter, Clone)]
pub enum IgnoredKind {
    Author = 0,
    Label = 1,
}

#[derive(Clone)]
pub struct IgnoreCriteria {
    db: Pool<Sqlite>,
    criteria: Arc<RwLock<Vec<IgnoreCriterion>>>,
}

#[derive(Clone)]
pub struct IgnoreCriterion {
    pub id: i64,
    pub kind: IgnoredKind,
    pub value: String,
}

impl IgnoreCriteria {
    pub async fn from_db(db: Pool<Sqlite>) -> Result<Self, sqlx::Error> {
        let criteria: Vec<IgnoreCriterion> =
            match query!("SELECT * FROM cr_ignored").fetch_all(&db).await {
                Ok(x) => x,
                Err(e) => {
                    error!("Failed to fetch CR ignore criteria from DB: {e}");
                    return Err(e)?;
                }
            }
            .iter()
            .filter_map(|x| {
                let Some(kind) = IgnoredKind::from_repr(x.kind) else {
                    error!("Fetched IgnoreCriterion with invalid kind {} from cr_ignored (ID {}, Value '{}'). Skipping.", x.kind, x.id, x.value);
                    return None;
                };
                Some(IgnoreCriterion {
                    id: x.id,
                    kind,
                    value: x.value.clone(),
                })
            })
            .collect();

        Ok(Self {
            db,
            criteria: Arc::new(RwLock::new(criteria)),
        })
    }

    pub async fn add(
        &self,
        kind: IgnoredKind,
        value: impl Into<String>,
    ) -> Result<(), HandledError> {
        let kind = kind as i64;
        let value = value.into();

        let criterion = match query!(
            "INSERT INTO cr_ignored (kind, value) VALUES (?1, ?2) RETURNING *",
            kind,
            value
        )
        .fetch_one(&self.db)
        .await
        {
            Ok(x) => IgnoreCriterion {
                id: x.id,
                kind: IgnoredKind::from_repr(x.kind).expect(
                    "We just wrote this after casting from the enum. This should be valid.",
                ),
                value: x.value,
            },
            Err(e) => {
                error!("Failed to insert new IgnoreCriterion into database: {e}");
                return Err(HandledError::InternalError);
            }
        };

        self.criteria.write().await.push(criterion);

        Ok(())
    }

    pub async fn remove(&self, id: i64) -> Result<(), HandledError> {
        if let Err(e) = query!("DELETE FROM cr_ignored WHERE id = ?1", id)
            .execute(&self.db)
            .await
        {
            error!("Failed to delete IgnoreCriterion with ID {id} from database: {e}");
            return Err(HandledError::InternalError);
        }

        let mut criteria = self.criteria.write().await;
        let idx = criteria.iter().position(|x| x.id == id);

        if let Some(idx) = idx {
            criteria.remove(idx);
        }

        Ok(())
    }

    pub async fn get_all(&self) -> Vec<IgnoreCriterion> {
        self.criteria.read().await.clone()
    }

    pub async fn is_ignored(&self, kind: IgnoredKind, value: impl AsRef<str>) -> bool {
        self.criteria
            .read()
            .await
            .iter()
            .find(|x| x.kind == kind && x.value == value.as_ref())
            .is_some()
    }
}
