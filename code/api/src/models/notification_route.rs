//! Which external channels a notification event is delivered to.
//!
//! A missing row means the route is off. The in-app bell does not consult routes.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "notification_routes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub event_type: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub channel_type: String,
    pub enabled: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
