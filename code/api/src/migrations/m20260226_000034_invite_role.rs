//! Migration: let an invite carry the role its new user receives.
//!
//! No foreign key: SQLite cannot add one to an existing table. Registration
//! ignores a role that no longer exists.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("invites"))
                    .add_column(ColumnDef::new(Alias::new("role_id")).big_integer().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("invites"))
                    .drop_column(Alias::new("role_id"))
                    .to_owned(),
            )
            .await
    }
}
