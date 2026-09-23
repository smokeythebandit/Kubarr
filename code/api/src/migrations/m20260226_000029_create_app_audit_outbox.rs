use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Outbox::Table)
                    .col(
                        ColumnDef::new(Outbox::OperationId)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Outbox::Action).string().not_null())
                    .col(ColumnDef::new(Outbox::ResourceType).string().not_null())
                    .col(ColumnDef::new(Outbox::ResourceId).string().not_null())
                    .col(ColumnDef::new(Outbox::CreatedBy).big_integer().null())
                    .col(ColumnDef::new(Outbox::Username).string().null())
                    .col(ColumnDef::new(Outbox::Details).string().not_null())
                    .col(ColumnDef::new(Outbox::Success).boolean().not_null())
                    .col(ColumnDef::new(Outbox::Outcome).string().not_null())
                    .col(ColumnDef::new(Outbox::ErrorLabel).string().null())
                    .col(
                        ColumnDef::new(Outbox::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Outbox::ProcessedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Outbox::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[iden = "app_audit_outbox"]
enum Outbox {
    Table,
    #[iden = "operation_id"]
    OperationId,
    Action,
    #[iden = "resource_type"]
    ResourceType,
    #[iden = "resource_id"]
    ResourceId,
    #[iden = "created_by"]
    CreatedBy,
    Username,
    Details,
    Success,
    Outcome,
    #[iden = "error_label"]
    ErrorLabel,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "processed_at"]
    ProcessedAt,
}
