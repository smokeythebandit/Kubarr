//! Migration: Create audit_logs table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AuditLogs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AuditLogs::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AuditLogs::Timestamp)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AuditLogs::UserId).big_integer().null())
                    .col(ColumnDef::new(AuditLogs::Username).string().null())
                    .col(ColumnDef::new(AuditLogs::Action).string().not_null())
                    .col(ColumnDef::new(AuditLogs::ResourceType).string().not_null())
                    .col(ColumnDef::new(AuditLogs::ResourceId).string().null())
                    .col(ColumnDef::new(AuditLogs::Details).string().null())
                    .col(ColumnDef::new(AuditLogs::IpAddress).string().null())
                    .col(ColumnDef::new(AuditLogs::UserAgent).string().null())
                    .col(
                        ColumnDef::new(AuditLogs::Success)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(ColumnDef::new(AuditLogs::ErrorMessage).string().null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_audit_logs_timestamp")
                    .table(AuditLogs::Table)
                    .col(AuditLogs::Timestamp)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_audit_logs_user_id")
                    .table(AuditLogs::Table)
                    .col(AuditLogs::UserId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_audit_logs_action")
                    .table(AuditLogs::Table)
                    .col(AuditLogs::Action)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_audit_logs_resource_type")
                    .table(AuditLogs::Table)
                    .col(AuditLogs::ResourceType)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AuditLogs::Table).if_exists().to_owned())
            .await
    }
}

#[derive(Iden)]
#[iden = "audit_logs"]
enum AuditLogs {
    Table,
    Id,
    Timestamp,
    #[iden = "user_id"]
    UserId,
    Username,
    Action,
    #[iden = "resource_type"]
    ResourceType,
    #[iden = "resource_id"]
    ResourceId,
    Details,
    #[iden = "ip_address"]
    IpAddress,
    #[iden = "user_agent"]
    UserAgent,
    Success,
    #[iden = "error_message"]
    ErrorMessage,
}
