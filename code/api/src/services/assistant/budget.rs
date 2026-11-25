//! Usage ledger and the monthly budget.
//!
//! Every Messages API call is recorded with its token counts and cost. The
//! budget is per install and per UTC calendar month: at 80 % the people who
//! manage settings get one notification per month, at 100 % new requests are
//! refused (a request already running is allowed to finish).

use chrono::{DateTime, Datelike, TimeZone, Utc};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, Set,
};

use super::client::Usage;
use super::pricing::AssistantModel;
use super::settings::{self, micros_to_usd, AssistantSettings};
use crate::error::{AppError, Result};
use crate::models::audit_log::AuditAction;
use crate::models::{assistant_usage, system_setting};
use crate::services::notification::NotificationService;

/// `system_settings` row holding the month (`YYYY-MM`) the 80 % warning was sent.
pub const KEY_WARNED_MONTH: &str = "assistant.budget_warned_month";

/// Start of the UTC calendar month containing `now`.
pub fn month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(now)
}

/// `YYYY-MM` of `now` in UTC.
pub fn month_label(now: DateTime<Utc>) -> String {
    now.format("%Y-%m").to_string()
}

/// Spend so far in the month containing `now`, in micro-dollars.
pub async fn month_spend_micros<C: ConnectionTrait>(db: &C, now: DateTime<Utc>) -> Result<i64> {
    let total: Option<i64> = assistant_usage::Entity::find()
        .select_only()
        .column_as(
            Expr::cust("CAST(COALESCE(SUM(cost_usd_micros), 0) AS BIGINT)"),
            "total",
        )
        .filter(assistant_usage::Column::CreatedAt.gte(month_start(now)))
        .into_tuple::<i64>()
        .one(db)
        .await?;
    Ok(total.unwrap_or(0))
}

/// This month's spend against the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetState {
    pub spend_usd_micros: i64,
    pub budget_usd_micros: i64,
}

impl BudgetState {
    /// The budget is used up: new requests are refused.
    pub fn paused(&self) -> bool {
        self.spend_usd_micros >= self.budget_usd_micros
    }

    /// Spend has reached 80 % of the budget.
    pub fn at_warning_level(&self) -> bool {
        // spend / budget >= 0.8, in integers.
        i128::from(self.spend_usd_micros) * 5 >= i128::from(self.budget_usd_micros) * 4
    }

    pub fn spend_usd(&self) -> f64 {
        micros_to_usd(self.spend_usd_micros)
    }
}

pub async fn budget_state<C: ConnectionTrait>(
    db: &C,
    settings: &AssistantSettings,
    now: DateTime<Utc>,
) -> Result<BudgetState> {
    Ok(BudgetState {
        spend_usd_micros: month_spend_micros(db, now).await?,
        budget_usd_micros: settings.budget_usd_micros(),
    })
}

/// Refuse a new request once this month's budget is used up.
pub async fn ensure_budget<C: ConnectionTrait>(db: &C) -> Result<()> {
    let settings = settings::load(db).await?;
    if budget_state(db, &settings, Utc::now()).await?.paused() {
        return Err(AppError::ServiceUnavailable(
            "The assistant's monthly budget is used up. It resumes next month, \
             or when an administrator raises the budget."
                .into(),
        ));
    }
    Ok(())
}

/// One Messages API call to record.
#[derive(Debug, Clone)]
pub struct UsageRecord {
    pub user_id: Option<i64>,
    pub conversation_id: Option<String>,
    pub model: AssistantModel,
    pub usage: Usage,
}

fn tokens(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// Store a call's usage and cost, then send the 80 % warning if it is due.
/// Returns the call's cost in micro-dollars.
pub async fn record_usage(
    db: &sea_orm::DatabaseConnection,
    notifications: &NotificationService,
    record: UsageRecord,
) -> Result<i64> {
    let now = Utc::now();
    let cost = record.model.cost_usd_micros(&record.usage);
    assistant_usage::ActiveModel {
        user_id: Set(record.user_id),
        conversation_id: Set(record.conversation_id),
        model: Set(record.model.id().to_string()),
        input_tokens: Set(tokens(record.usage.input_tokens)),
        output_tokens: Set(tokens(record.usage.output_tokens)),
        cache_read_tokens: Set(tokens(record.usage.cache_read_input_tokens.unwrap_or(0))),
        cache_write_tokens: Set(tokens(
            record.usage.cache_creation_input_tokens.unwrap_or(0),
        )),
        cost_usd_micros: Set(cost),
        created_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await?;
    if let Err(e) = warn_if_due(db, notifications, now).await {
        tracing::warn!("Assistant budget warning check failed: {e}");
    }
    Ok(cost)
}

/// Notify settings managers once per month when spend reaches 80 % of the
/// budget. Returns whether this call sent the warning.
pub async fn warn_if_due(
    db: &sea_orm::DatabaseConnection,
    notifications: &NotificationService,
    now: DateTime<Utc>,
) -> Result<bool> {
    let settings = settings::load(db).await?;
    let state = budget_state(db, &settings, now).await?;
    if !state.at_warning_level() || !claim_warning(db, now).await? {
        return Ok(false);
    }
    let details = format!(
        "${:.2} of ${:.2} spent this month",
        state.spend_usd(),
        settings.monthly_budget_usd
    );
    notifications
        .notify_event(
            &AuditAction::AssistantBudgetWarning,
            None,
            None,
            Some(&details),
        )
        .await?;
    Ok(true)
}

/// Atomically mark this month as warned; false when it already was (by this
/// or another replica).
async fn claim_warning<C: ConnectionTrait>(db: &C, now: DateTime<Utc>) -> Result<bool> {
    let month = month_label(now);
    let updated = system_setting::Entity::update_many()
        .col_expr(system_setting::Column::Value, Expr::value(month.clone()))
        .col_expr(system_setting::Column::UpdatedAt, Expr::value(now))
        .filter(system_setting::Column::Key.eq(KEY_WARNED_MONTH))
        .filter(system_setting::Column::Value.ne(month.clone()))
        .exec(db)
        .await?;
    if updated.rows_affected > 0 {
        return Ok(true);
    }
    let inserted = system_setting::Entity::insert(system_setting::ActiveModel {
        key: Set(KEY_WARNED_MONTH.into()),
        value: Set(month),
        description: Set(Some("Month the assistant budget warning was sent".into())),
        updated_at: Set(now),
    })
    .on_conflict(
        OnConflict::column(system_setting::Column::Key)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(inserted > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{role, user, user_notification, user_role};
    use sea_orm::{Database, DatabaseConnection, PaginatorTrait};
    use sea_orm_migration::MigratorTrait;

    async fn db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::migrations::Migrator::up(&db, None).await.unwrap();
        db
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    async fn spend(db: &DatabaseConnection, micros: i64, at: DateTime<Utc>) {
        assistant_usage::ActiveModel {
            model: Set("claude-opus-5-5".into()),
            cost_usd_micros: Set(micros),
            created_at: Set(at),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn admin(db: &DatabaseConnection) -> i64 {
        let now = Utc::now();
        let user = user::ActiveModel {
            username: Set("owner".into()),
            email: Set("owner@example.com".into()),
            hashed_password: Set("x".into()),
            is_active: Set(true),
            is_approved: Set(true),
            totp_enabled: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
        let role = role::Entity::find()
            .filter(role::Column::Name.eq("admin"))
            .one(db)
            .await
            .unwrap()
            .unwrap();
        user_role::ActiveModel {
            user_id: Set(user.id),
            role_id: Set(role.id),
        }
        .insert(db)
        .await
        .unwrap();
        user.id
    }

    async fn notifier(db: &DatabaseConnection) -> NotificationService {
        let service = NotificationService::new();
        service.set_db(db.clone()).await;
        service
    }

    async fn warnings_for(db: &DatabaseConnection, user_id: i64) -> u64 {
        user_notification::Entity::find()
            .filter(user_notification::Column::UserId.eq(user_id))
            .filter(user_notification::Column::EventType.eq("assistant_budget_warning"))
            .count(db)
            .await
            .unwrap()
    }

    #[test]
    fn month_bounds_are_utc_calendar_months() {
        let now = utc("2026-11-25T13:14:15Z");
        assert_eq!(month_start(now), utc("2026-11-01T00:00:00Z"));
        assert_eq!(month_label(now), "2026-11");
    }

    #[tokio::test]
    async fn spend_counts_only_the_current_month() {
        let db = db().await;
        let now = utc("2026-11-25T12:00:00Z");
        assert_eq!(month_spend_micros(&db, now).await.unwrap(), 0);
        spend(&db, 1_500_000, utc("2026-10-31T23:59:59Z")).await;
        spend(&db, 2_000_000, utc("2026-11-01T00:00:00Z")).await;
        spend(&db, 250_000, utc("2026-11-20T08:00:00Z")).await;
        assert_eq!(month_spend_micros(&db, now).await.unwrap(), 2_250_000);
    }

    #[test]
    fn pause_and_warning_thresholds() {
        let state = |spend| BudgetState {
            spend_usd_micros: spend,
            budget_usd_micros: 20_000_000,
        };
        assert!(!state(15_999_999).at_warning_level());
        assert!(state(16_000_000).at_warning_level());
        assert!(!state(19_999_999).paused());
        assert!(state(20_000_000).paused());
        assert!(state(25_000_000).paused());
        // A zero budget keeps the assistant paused.
        let zero = BudgetState {
            spend_usd_micros: 0,
            budget_usd_micros: 0,
        };
        assert!(zero.paused());
    }

    #[tokio::test]
    async fn ensure_budget_pauses_at_100_percent() {
        let db = db().await;
        ensure_budget(&db).await.unwrap();
        spend(&db, 19_999_999, Utc::now()).await;
        ensure_budget(&db).await.unwrap();
        spend(&db, 1, Utc::now()).await;
        assert!(matches!(
            ensure_budget(&db).await,
            Err(AppError::ServiceUnavailable(_))
        ));
        // Raising the budget resumes it.
        settings::save(
            &db,
            &AssistantSettings {
                monthly_budget_usd: 50.0,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
        ensure_budget(&db).await.unwrap();
    }

    #[tokio::test]
    async fn record_usage_stores_tokens_and_cost() {
        let db = db().await;
        let cost = record_usage(
            &db,
            &notifier(&db).await,
            UsageRecord {
                user_id: Some(7),
                conversation_id: Some("c1".into()),
                model: AssistantModel::Sonnet55,
                usage: Usage {
                    input_tokens: 1_000,
                    output_tokens: 100,
                    cache_read_input_tokens: Some(10_000),
                    cache_creation_input_tokens: Some(400),
                },
            },
        )
        .await
        .unwrap();
        // 1000*2000 + 100*10000 + 10000*100 + 400*2500 nanos = 5 000 µ$.
        assert_eq!(cost, 5_000);
        let row = assistant_usage::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.user_id, Some(7));
        assert_eq!(row.model, "claude-sonnet-5-5");
        assert_eq!(
            (
                row.input_tokens,
                row.output_tokens,
                row.cache_read_tokens,
                row.cache_write_tokens
            ),
            (1_000, 100, 10_000, 400)
        );
        assert_eq!(row.cost_usd_micros, 5_000);
    }

    #[tokio::test]
    async fn warning_is_sent_once_per_month_at_80_percent() {
        let db = db().await;
        let owner = admin(&db).await;
        let notifications = notifier(&db).await;
        let november = utc("2026-11-10T10:00:00Z");

        spend(&db, 15_000_000, november).await;
        assert!(!warn_if_due(&db, &notifications, november).await.unwrap());
        assert_eq!(warnings_for(&db, owner).await, 0);

        spend(&db, 1_000_000, november).await;
        assert!(warn_if_due(&db, &notifications, november).await.unwrap());
        assert_eq!(warnings_for(&db, owner).await, 1);

        // More spend in the same month: no second warning.
        spend(&db, 2_000_000, november).await;
        assert!(!warn_if_due(&db, &notifications, november).await.unwrap());
        assert_eq!(warnings_for(&db, owner).await, 1);

        // Next month starts fresh and warns again once the level is reached.
        let december = utc("2026-12-03T10:00:00Z");
        assert!(!warn_if_due(&db, &notifications, december).await.unwrap());
        spend(&db, 17_000_000, december).await;
        assert!(warn_if_due(&db, &notifications, december).await.unwrap());
        assert!(!warn_if_due(&db, &notifications, december).await.unwrap());
        assert_eq!(warnings_for(&db, owner).await, 2);
    }
}
