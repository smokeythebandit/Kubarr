use kubarr::migrations::Migrator;
use kubarr::models::app_storage_config;
use kubarr::services::{
    app_storage,
    install_options::{
        InstallOptions, InstallStorage, Mount, MountMode, PrivateConfig, WorkloadMounts,
    },
};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, Database, EntityTrait, Set, Statement, TransactionTrait,
};
use sea_orm_migration::{MigrationName, MigratorTrait};
use std::collections::BTreeMap;

fn mount(source: &str) -> Mount {
    Mount {
        name: "data".into(),
        source: source.into(),
        target: "/data".into(),
        mode: MountMode::ReadWrite,
    }
}
fn options(source: &str) -> InstallOptions {
    InstallOptions {
        app_name: "fixture".into(),
        chart_version: "2.0.0".into(),
        mounts: Some(vec![mount(source)]),
        workloads: BTreeMap::from([(
            "worker".into(),
            vec![Mount {
                name: "logs".into(),
                source: "logs/new".into(),
                target: "/logs".into(),
                mode: MountMode::ReadWrite,
            }],
        )]),
        config: Some(PrivateConfig {
            create: true,
            existing_claim: "".into(),
            storage_class_name: "".into(),
            size: "5Gi".into(),
        }),
        private_claim: Some("fixture-config".into()),
        required: BTreeMap::new(),
        reserved: BTreeMap::new(),
        vpn_supported: false,
        gpu_supported: false,
        saved_storage: None,
        saved_chart_version: None,
    }
}

#[tokio::test]
async fn sqlite_upgrade_saved_configuration_merges_defaults_and_rolls_back_atomically() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    // Apply everything up to, but not including, the migration under test.
    // Pinned by name: counting from the end breaks whenever a later migration
    // is added.
    let before_storage = Migrator::migrations()
        .iter()
        .position(|migration| migration.name().contains("create_app_storage_configs"))
        .expect("app storage migration is registered") as u32;
    Migrator::up(&db, Some(before_storage)).await.unwrap();
    let before = db
        .query_one_raw(Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE name='app_storage_configs'",
        ))
        .await
        .unwrap();
    assert!(before.is_none());
    Migrator::up(&db, None).await.unwrap();
    assert!(app_storage::get(&db, "fixture").await.unwrap().is_none());

    let initial =
        app_storage::materialize(&options("media/new"), &InstallStorage::default(), None).unwrap();
    assert_eq!(initial.mounts.as_ref().unwrap()[0].source, "media/new");
    assert_eq!(
        initial.workloads.as_ref().unwrap()["worker"].mounts[0].source,
        "logs/new"
    );
    let tx = db.begin().await.unwrap();
    app_storage::save(&tx, "fixture", "1.0.0", &initial)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(app_storage::get(&db, "fixture").await.unwrap().is_none());

    app_storage::save(&db, "fixture", "1.0.0", &initial)
        .await
        .unwrap();
    let saved = app_storage::get(&db, "fixture").await.unwrap().unwrap();
    assert_eq!(saved.chart_version, "1.0.0");
    assert!(!saved.fresh_private_config);
    app_storage::mark_fresh_private_config(&db, "fixture", "1.0.0")
        .await
        .unwrap();
    assert!(
        app_storage::get(&db, "fixture")
            .await
            .unwrap()
            .unwrap()
            .fresh_private_config
    );
    let next = options("media/changed-default");
    // Omission preserves the saved lists and config, even when chart defaults change.
    let retained =
        app_storage::materialize(&next, &InstallStorage::default(), Some(&saved.storage)).unwrap();
    assert_eq!(retained.mounts.as_ref().unwrap()[0].source, "media/new");
    assert_eq!(
        retained.workloads.as_ref().unwrap()["worker"].mounts[0].source,
        "logs/new"
    );
    assert_eq!(retained.config.as_ref().unwrap().size, "5Gi");

    let empty = app_storage::materialize(
        &next,
        &InstallStorage {
            mounts: Some(vec![]),
            ..Default::default()
        },
        Some(&saved.storage),
    )
    .unwrap();
    assert!(empty.mounts.as_ref().unwrap().is_empty());
    assert_eq!(empty.workloads.as_ref().unwrap()["worker"].mounts.len(), 1);
    app_storage::save(&db, "fixture", "2.0.0", &empty)
        .await
        .unwrap();
    let updated = app_storage::get(&db, "fixture").await.unwrap().unwrap();
    assert_eq!(updated.chart_version, "2.0.0");
    assert_eq!(updated.created_at, saved.created_at);
    assert!(
        updated.fresh_private_config,
        "changing mounts preserves proof of fresh private config"
    );
    assert!(updated.storage.mounts.unwrap().is_empty());

    let changed_claim = app_storage::materialize(
        &next,
        &InstallStorage {
            config: Some(PrivateConfig {
                create: false,
                existing_claim: "another-config".into(),
                storage_class_name: "".into(),
                size: "5Gi".into(),
            }),
            ..Default::default()
        },
        Some(&saved.storage),
    )
    .unwrap();
    app_storage::save(&db, "fixture", "2.0.0", &changed_claim)
        .await
        .unwrap();
    assert!(
        !app_storage::get(&db, "fixture")
            .await
            .unwrap()
            .unwrap()
            .fresh_private_config,
        "changing the private claim revokes automatic future-upgrade confirmation"
    );

    let row = app_storage_config::Entity::find_by_id("fixture")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let mut corrupt: app_storage_config::ActiveModel = row.into();
    corrupt.config_json = Set("{\"storage.migrationConfirmed\":true}".into());
    corrupt.update(&db).await.unwrap();
    assert!(app_storage::get(&db, "fixture").await.is_err());
    let mut unknown: app_storage_config::ActiveModel =
        app_storage_config::Entity::find_by_id("fixture")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
    unknown.schema_version = Set(99);
    unknown.update(&db).await.unwrap();
    assert!(app_storage::get(&db, "fixture")
        .await
        .unwrap_err()
        .to_string()
        .contains("Unsupported saved storage schema"));
    // Roll back far enough to undo the migration under test, which is no longer
    // the most recent one.
    let through_storage = Migrator::migrations().len() as u32 - before_storage;
    Migrator::down(&db, Some(through_storage)).await.unwrap();
    assert!(db
        .query_one_raw(Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE name='app_storage_configs'"
        ))
        .await
        .unwrap()
        .is_none());
}

#[test]
fn saved_workloads_cannot_disappear_or_bypass_required_mounts() {
    let mut old = options("media/original");
    old.required.insert("worker".into(), vec!["/logs".into()]);
    let initial = app_storage::materialize(&old, &InstallStorage::default(), None).unwrap();
    assert!(app_storage::materialize(
        &old,
        &InstallStorage {
            workloads: Some(BTreeMap::from([(
                "worker".into(),
                WorkloadMounts { mounts: vec![] }
            )])),
            ..Default::default()
        },
        Some(&initial)
    )
    .is_err());
    let mut new = old;
    new.workloads.clear();
    assert!(app_storage::materialize(&new, &InstallStorage::default(), Some(&initial)).is_err());
}
