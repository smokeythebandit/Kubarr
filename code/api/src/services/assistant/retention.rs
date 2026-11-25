//! Leader task deleting conversations older than `assistant.retention_days`.

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::DatabaseConnection;

use super::{conversations, settings};
use crate::services::scheduler::PeriodicTask;

pub struct AssistantRetentionTask;

#[async_trait]
impl PeriodicTask for AssistantRetentionTask {
    fn name(&self) -> &'static str {
        "assistant_retention"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(60 * 60)
    }

    async fn run(&self, db: &DatabaseConnection) -> anyhow::Result<()> {
        let settings = settings::load(db).await?;
        let deleted =
            conversations::delete_expired(db, settings.retention_days, Utc::now()).await?;
        if deleted > 0 {
            tracing::info!(deleted, "Assistant conversation retention completed");
        }
        Ok(())
    }
}
