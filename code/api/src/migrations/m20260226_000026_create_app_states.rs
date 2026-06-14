//! Migration: Create app_states table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AppStates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppStates::AppName)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AppStates::Namespace).string().not_null())
                    .col(ColumnDef::new(AppStates::DesiredState).string().not_null())
                    .col(ColumnDef::new(AppStates::ObservedState).string().not_null())
                    .col(
                        ColumnDef::new(AppStates::Healthy)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(AppStates::Message).text().null())
                    .col(
                        ColumnDef::new(AppStates::InstalledChartVersion)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppStates::AvailableChartVersion)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppStates::UpdateAvailable)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(AppStates::LastOperationId).string().null())
                    .col(
                        ColumnDef::new(AppStates::LastCheckedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppStates::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AppStates::Table).if_exists().to_owned())
            .await
    }
}

#[derive(Iden)]
#[iden = "app_states"]
enum AppStates {
    Table,
    #[iden = "app_name"]
    AppName,
    Namespace,
    #[iden = "desired_state"]
    DesiredState,
    #[iden = "observed_state"]
    ObservedState,
    Healthy,
    Message,
    #[iden = "installed_chart_version"]
    InstalledChartVersion,
    #[iden = "available_chart_version"]
    AvailableChartVersion,
    #[iden = "update_available"]
    UpdateAvailable,
    #[iden = "last_operation_id"]
    LastOperationId,
    #[iden = "last_checked_at"]
    LastCheckedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
