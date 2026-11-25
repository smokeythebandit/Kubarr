//! Owner-facing assistant settings, stored in `system_settings` under the
//! `assistant.` prefix. Absent or unreadable keys fall back to the defaults.

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set,
    TransactionSession, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use super::pricing::AssistantModel;
use crate::error::{AppError, Result};
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::models::system_setting;
use crate::services::audit::log_on_transaction;

pub const KEY_ENABLED: &str = "assistant.enabled";
pub const KEY_MODEL: &str = "assistant.model";
pub const KEY_EFFORT: &str = "assistant.effort";
pub const KEY_MONTHLY_BUDGET: &str = "assistant.monthly_budget_usd";
pub const KEY_RETENTION_DAYS: &str = "assistant.retention_days";

const MAX_MONTHLY_BUDGET_USD: f64 = 100_000.0;
const RETENTION_DAYS: std::ops::RangeInclusive<u32> = 1..=365;

/// How much the model thinks before answering (`output_config.effort`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Effort::Low, Effort::Medium, Effort::High]
            .into_iter()
            .find(|effort| effort.as_str() == value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AssistantSettings {
    /// Whether people with `assistant.use` may chat (the key must also be set).
    pub enabled: bool,
    pub model: AssistantModel,
    pub effort: Effort,
    /// Monthly spending cap in US dollars; the assistant pauses when reached.
    pub monthly_budget_usd: f64,
    /// Days conversations are kept.
    pub retention_days: u32,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: AssistantModel::Opus55,
            effort: Effort::Low,
            monthly_budget_usd: 20.0,
            retention_days: 30,
        }
    }
}

impl AssistantSettings {
    /// Check every field, naming the offending one in the error.
    pub fn validate(&self) -> Result<()> {
        if !self.monthly_budget_usd.is_finite()
            || !(0.0..=MAX_MONTHLY_BUDGET_USD).contains(&self.monthly_budget_usd)
        {
            return Err(AppError::BadRequest(format!(
                "monthly_budget_usd: must be between 0 and {MAX_MONTHLY_BUDGET_USD}"
            )));
        }
        if !RETENTION_DAYS.contains(&self.retention_days) {
            return Err(AppError::BadRequest(format!(
                "retention_days: must be between {} and {}",
                RETENTION_DAYS.start(),
                RETENTION_DAYS.end()
            )));
        }
        Ok(())
    }

    /// The budget in micro-dollars, the unit of the usage ledger.
    pub fn budget_usd_micros(&self) -> i64 {
        usd_to_micros(self.monthly_budget_usd)
    }
}

pub fn usd_to_micros(usd: f64) -> i64 {
    (usd * 1_000_000.0).round() as i64
}

pub fn micros_to_usd(micros: i64) -> f64 {
    micros as f64 / 1_000_000.0
}

/// Load the settings; keys never saved (or unreadable) use their defaults.
pub async fn load<C: ConnectionTrait>(db: &C) -> Result<AssistantSettings> {
    let mut settings = AssistantSettings::default();
    let rows = system_setting::Entity::find()
        .filter(system_setting::Column::Key.starts_with("assistant."))
        .all(db)
        .await?;
    for row in rows {
        let value = row.value.as_str();
        match row.key.as_str() {
            KEY_ENABLED => settings.enabled = value.parse().unwrap_or(settings.enabled),
            KEY_MODEL => settings.model = AssistantModel::parse(value).unwrap_or(settings.model),
            KEY_EFFORT => settings.effort = Effort::parse(value).unwrap_or(settings.effort),
            KEY_MONTHLY_BUDGET => {
                settings.monthly_budget_usd = value
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .unwrap_or(settings.monthly_budget_usd)
            }
            KEY_RETENTION_DAYS => {
                settings.retention_days = value
                    .parse()
                    .ok()
                    .filter(|v| RETENTION_DAYS.contains(v))
                    .unwrap_or(settings.retention_days)
            }
            _ => {}
        }
    }
    Ok(settings)
}

/// Upsert one `system_settings` row.
pub(crate) async fn put_setting<C: ConnectionTrait>(
    db: &C,
    key: &str,
    value: String,
    description: &str,
) -> Result<()> {
    let now = Utc::now();
    match system_setting::Entity::find_by_id(key).one(db).await? {
        Some(existing) => {
            let mut row: system_setting::ActiveModel = existing.into();
            row.value = Set(value);
            row.updated_at = Set(now);
            row.update(db).await?;
        }
        None => {
            system_setting::ActiveModel {
                key: Set(key.into()),
                value: Set(value),
                description: Set(Some(description.into())),
                updated_at: Set(now),
            }
            .insert(db)
            .await?;
        }
    }
    Ok(())
}

/// Validate and persist all settings in one transaction, recording who changed them.
pub async fn save(
    db: &impl TransactionTrait,
    settings: &AssistantSettings,
    actor: Option<(i64, String)>,
) -> Result<()> {
    settings.validate()?;
    let txn = db.begin().await?;
    let rows = [
        (KEY_ENABLED, settings.enabled.to_string()),
        (KEY_MODEL, settings.model.id().to_string()),
        (KEY_EFFORT, settings.effort.as_str().to_string()),
        (KEY_MONTHLY_BUDGET, settings.monthly_budget_usd.to_string()),
        (KEY_RETENTION_DAYS, settings.retention_days.to_string()),
    ];
    for (key, value) in rows {
        put_setting(&txn, key, value, "AI assistant").await?;
    }
    let (user_id, username) = actor.unzip();
    log_on_transaction(
        &txn,
        AuditAction::SystemSettingChanged,
        ResourceType::System,
        Some("assistant".into()),
        user_id,
        username,
        None,
        None,
        None,
        true,
        None,
    )
    .await?;
    txn.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    async fn db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::migrations::Migrator::up(&db, None).await.unwrap();
        db
    }

    #[tokio::test]
    async fn defaults_when_nothing_saved() {
        let settings = load(&db().await).await.unwrap();
        assert_eq!(settings, AssistantSettings::default());
        assert_eq!(settings.model, AssistantModel::Opus55);
        assert_eq!(settings.effort, Effort::Low);
        assert_eq!(settings.monthly_budget_usd, 20.0);
        assert_eq!(settings.retention_days, 30);
        assert!(settings.enabled);
    }

    #[tokio::test]
    async fn save_round_trips_and_audits() {
        let db = db().await;
        let changed = AssistantSettings {
            enabled: false,
            model: AssistantModel::Haiku55,
            effort: Effort::High,
            monthly_budget_usd: 7.5,
            retention_days: 90,
        };
        save(&db, &changed, Some((1, "owner".into())))
            .await
            .unwrap();
        assert_eq!(load(&db).await.unwrap(), changed);
        let audit = crate::models::audit_log::Entity::find()
            .all(&db)
            .await
            .unwrap();
        assert!(audit
            .iter()
            .any(|row| row.action == "system_setting_changed"
                && row.resource_id.as_deref() == Some("assistant")));
    }

    #[tokio::test]
    async fn invalid_values_are_rejected_with_field_names() {
        let db = db().await;
        for (settings, field) in [
            (
                AssistantSettings {
                    monthly_budget_usd: -1.0,
                    ..Default::default()
                },
                "monthly_budget_usd",
            ),
            (
                AssistantSettings {
                    monthly_budget_usd: f64::NAN,
                    ..Default::default()
                },
                "monthly_budget_usd",
            ),
            (
                AssistantSettings {
                    retention_days: 0,
                    ..Default::default()
                },
                "retention_days",
            ),
        ] {
            let err = save(&db, &settings, None).await.unwrap_err().to_string();
            assert!(err.contains(field), "{err}");
        }
    }

    #[tokio::test]
    async fn unreadable_stored_values_fall_back_to_defaults() {
        let db = db().await;
        put_setting(&db, KEY_MODEL, "gpt-x".into(), "")
            .await
            .unwrap();
        put_setting(&db, KEY_MONTHLY_BUDGET, "lots".into(), "")
            .await
            .unwrap();
        assert_eq!(load(&db).await.unwrap(), AssistantSettings::default());
    }

    #[test]
    fn dollars_convert_to_micros() {
        assert_eq!(usd_to_micros(20.0), 20_000_000);
        assert_eq!(usd_to_micros(0.1), 100_000);
        assert_eq!(micros_to_usd(16_500_000), 16.5);
    }
}
