use chrono::{Days, Utc};
use poise::serenity_prelude::ChannelId;
use sqlx::{Pool, Sqlite, query};
use tracing::{error, warn};

use crate::discord::content_review::HandledError;

#[derive(Default, Debug, Clone)]
pub struct DiscussionRecord {
    pub pr_id: u64,
    pub forum_id: ChannelId,
    pub thread_id: ChannelId,

    pub review_days_total: Option<u64>,
    pub review_days_passed: Option<u64>,
    pub review_days_next_micros: Option<i64>,

    pub pr_title: String,
    pub pr_author: String,
    pub pr_body: Option<String>,
}

impl DiscussionRecord {
    pub async fn get_all(db: &Pool<Sqlite>) -> Vec<Self> {
        match query!("SELECT * FROM cr_discussions").fetch_all(db).await {
            Ok(records) => records
                .iter()
                .map(|r| DiscussionRecord {
                    forum_id: ChannelId::new(r.forum_id.cast_unsigned()),
                    pr_id: r.pr_id.cast_unsigned(),
                    thread_id: ChannelId::new(r.thread_id.cast_unsigned()),
                    review_days_total: r.review_days_total.and_then(|x| Some(x.cast_unsigned())),
                    review_days_passed: r.review_days_passed.and_then(|x| Some(x.cast_unsigned())),
                    review_days_next_micros: r.review_days_next_micros,
                    pr_title: r.pr_title.clone(),
                    pr_author: r.pr_author.clone(),
                    pr_body: r.pr_body.clone(),
                    ..Default::default()
                })
                .collect(),

            Err(e) => {
                error!("Failed to get all discussion records: {e}");
                Vec::new()
            }
        }
    }

    pub async fn set_thread_id(
        &mut self,
        db: &Pool<Sqlite>,
        new_thread: ChannelId,
    ) -> Result<(), HandledError> {
        let new_thread_s = new_thread.get().cast_signed();
        let pr_id_s = self.pr_id.cast_signed();

        if let Err(e) = sqlx::query!(
            "UPDATE cr_discussions SET thread_id=?1 WHERE pr_id = ?2",
            new_thread_s,
            pr_id_s
        )
        .execute(db)
        .await
        {
            error!(
                "Failed to set new thread id {new_thread} for discussion of PR #{}: {e}",
                self.pr_id
            );

            return Err(HandledError::InternalError);
        }

        self.thread_id = new_thread;

        Ok(())
    }

    pub async fn set_forum_id(
        &mut self,
        db: &Pool<Sqlite>,
        new_forum: ChannelId,
    ) -> Result<(), HandledError> {
        let new_forum_s = new_forum.get().cast_signed();
        let pr_id_s = self.pr_id.cast_signed();

        if let Err(e) = sqlx::query!(
            "UPDATE cr_discussions SET forum_id=?1 WHERE pr_id = ?2",
            new_forum_s,
            pr_id_s
        )
        .execute(db)
        .await
        {
            error!(
                "Failed to set new forum id {new_forum} for discussion of PR #{}: {e}",
                self.pr_id
            );

            return Err(HandledError::InternalError);
        }

        self.forum_id = new_forum;

        Ok(())
    }

    pub async fn delete(&self, db: &Pool<Sqlite>) -> Result<(), HandledError> {
        let pr_id_s = self.pr_id.cast_signed();
        if let Err(e) = sqlx::query!("DELETE FROM cr_discussions WHERE pr_id = ?1", pr_id_s)
            .execute(db)
            .await
        {
            error!(
                "Failed to delete discussion record for pr #{}: {e}",
                self.pr_id
            );
            return Err(HandledError::InternalError);
        }

        Ok(())
    }

    pub async fn delete_body(&self, db: &Pool<Sqlite>) -> Result<(), HandledError> {
        let pr_id_s = self.pr_id.cast_signed();
        if let Err(e) = sqlx::query!(
            "UPDATE cr_discussions SET pr_body = NULL WHERE pr_id = ?1",
            pr_id_s
        )
        .execute(db)
        .await
        {
            error!("Failed to null PR body for pr #{}: {e}", self.pr_id);
            return Err(HandledError::InternalError);
        }

        Ok(())
    }

    pub async fn get_by_pr(db: &Pool<Sqlite>, pr_id: u64) -> Option<DiscussionRecord> {
        let pr_id_s = pr_id.cast_signed();
        match sqlx::query!("SELECT * FROM cr_discussions WHERE pr_id = ?1", pr_id_s)
            .fetch_one(db)
            .await
        {
            Ok(r) => Some(DiscussionRecord {
                forum_id: ChannelId::new(r.forum_id.cast_unsigned()),
                pr_id: r.pr_id.cast_unsigned(),
                thread_id: ChannelId::new(r.thread_id.cast_unsigned()),
                review_days_total: r.review_days_total.and_then(|x| Some(x.cast_unsigned())),
                review_days_passed: r.review_days_passed.and_then(|x| Some(x.cast_unsigned())),
                review_days_next_micros: r.review_days_next_micros,
                pr_title: r.pr_title,
                pr_author: r.pr_author,
                pr_body: r.pr_body,
                ..Default::default()
            }),
            Err(e) => {
                warn!("Failed to get discussion by PR#{pr_id}: {e}");
                None
            }
        }
    }

    pub async fn get_by_thread(
        db: &Pool<Sqlite>,
        thread_id: ChannelId,
    ) -> Option<DiscussionRecord> {
        let thread_id_s = thread_id.get().cast_signed();
        match sqlx::query!(
            "SELECT * FROM cr_discussions WHERE thread_id = ?1",
            thread_id_s
        )
        .fetch_one(db)
        .await
        {
            Ok(r) => Some(DiscussionRecord {
                forum_id: ChannelId::new(r.forum_id.cast_unsigned()),
                pr_id: r
                    .pr_id
                    .expect("primary key of record was null. this should not be possible.") // TODO: This suddenly started being returned as an Option. I have no idea why.
                    .cast_unsigned(),
                thread_id: ChannelId::new(r.thread_id.cast_unsigned()),
                review_days_total: r.review_days_total.and_then(|x| Some(x.cast_unsigned())),
                review_days_passed: r.review_days_passed.and_then(|x| Some(x.cast_unsigned())),
                review_days_next_micros: r.review_days_next_micros,
                pr_title: r.pr_title,
                pr_author: r.pr_author,
                pr_body: r.pr_body,
            }),
            Err(e) => {
                warn!("Failed to get discussion by thread {thread_id}: {e}");
                None
            }
        }
    }

    pub async fn insert(&self, db: &Pool<Sqlite>) -> Result<(), HandledError> {
        let pr_id = self.pr_id.cast_signed();
        let forum_id = self.forum_id.get().cast_signed();
        let thread_id = self.thread_id.get().cast_signed();
        let review_days_total = self.review_days_total.and_then(|x| Some(x.cast_signed()));
        let review_days_passed = self.review_days_passed.and_then(|x| Some(x.cast_signed()));

        match sqlx::query!(
            "INSERT INTO cr_discussions(pr_id, forum_id, thread_id, review_days_total, review_days_passed, review_days_next_micros, pr_title, pr_author, pr_body) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            pr_id,
            forum_id,
            thread_id,
            review_days_total,
            review_days_passed,
            self.review_days_next_micros,
            self.pr_title,
            self.pr_author,
            self.pr_body
        )
        .execute(db)
        .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Failed to insert CR discussion {self:?}: {e}");
                Err(HandledError::InternalError)
            }
        }
    }

    pub async fn setup_review_time(
        &mut self,
        db: &Pool<Sqlite>,
        days: u64,
    ) -> Result<(), HandledError> {
        let review_days_total_s = Some(days.cast_signed());
        let review_days_passed_s = Some(0i64);
        let review_days_next_micros = Self::get_next_micros();
        let pr_id_s = self.pr_id.cast_signed();

        match sqlx::query!(
            r#"UPDATE cr_discussions
            SET review_days_total=?1, review_days_passed=?2, review_days_next_micros=?3
            WHERE pr_id = ?4
            "#,
            review_days_total_s,
            review_days_passed_s,
            review_days_next_micros,
            pr_id_s
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                self.review_days_total = Some(days);
                self.review_days_passed = Some(0u64);
                self.review_days_next_micros = Some(review_days_next_micros);
                Ok(())
            }
            Err(e) => {
                error!("Failed to set up CR discussion review time {self:?}: {e}");
                Err(HandledError::InternalError)
            }
        }
    }

    pub async fn advance_review_timer(&mut self, db: &Pool<Sqlite>) -> Result<(), HandledError> {
        let review_days_passed = self.review_days_passed.unwrap_or(0) + 1;
        let review_days_passed_s = review_days_passed.cast_signed();
        let review_days_next_micros = Self::get_next_micros();
        let pr_id_s = self.pr_id.cast_signed();

        match sqlx::query!(
            r#"UPDATE cr_discussions
            SET review_days_passed=?1, review_days_next_micros=?2
            WHERE pr_id = ?3
            "#,
            review_days_passed_s,
            review_days_next_micros,
            pr_id_s
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                self.review_days_passed = Some(review_days_passed);
                self.review_days_next_micros = Some(review_days_next_micros);
                Ok(())
            }
            Err(e) => {
                error!("Failed to advance CR discussion review timer {self:?}: {e}");
                Err(HandledError::InternalError)
            }
        }
    }

    pub async fn disable_reminders(&mut self, db: &Pool<Sqlite>) -> Result<bool, HandledError> {
        if self.review_days_next_micros.is_none() {
            return Ok(false);
        }

        let pr_id_s = self.pr_id.cast_signed();

        match sqlx::query!(
            r#"UPDATE cr_discussions
                SET review_days_next_micros=NULL
                WHERE pr_id = ?1
                "#,
            pr_id_s
        )
        .execute(db)
        .await
        {
            Ok(_) => {
                self.review_days_next_micros = None;
                Ok(true)
            }
            Err(e) => {
                error!("Failed to null next day micros for discussion {self:?}: {e}");
                Err(HandledError::InternalError)
            }
        }
    }

    fn get_next_micros() -> i64 {
        let next_day = Utc::now().checked_add_days(Days::new(1)).unwrap();
        next_day.timestamp_micros()
    }
}
