mod common;

use common::create_test_db_with_seed;
use kubarr::services::deployment::media_storage_helm_values;
use kubarr::services::storage_config::{
    get_storage_config_from_db, save_storage_config_to_db, PersistedStorageConfig, StorageMode,
    STORAGE_MOUNT_PATH,
};

fn managed_storage_config() -> PersistedStorageConfig {
    PersistedStorageConfig {
        mode: StorageMode::ManagedNfs,
        mount_path: STORAGE_MOUNT_PATH.to_string(),
        uid: 1000,
        gid: 1000,
        fs_group: 1000,
        config_json: serde_json::json!({}),
        validation_json: None,
    }
}

#[tokio::test]
async fn storage_config_db_roundtrip_preserves_validation() {
    let db = create_test_db_with_seed().await;
    let config = PersistedStorageConfig {
        mode: StorageMode::ExternalNfs,
        mount_path: STORAGE_MOUNT_PATH.to_string(),
        uid: 1000,
        gid: 1000,
        fs_group: 1000,
        config_json: serde_json::json!({
            "server": "nas.local",
            "export_path": "/exports/media"
        }),
        validation_json: Some(serde_json::json!({
            "valid": true,
            "message": "ok"
        })),
    };

    save_storage_config_to_db(&db, &config)
        .await
        .expect("save storage config");
    let (fetched, _) = get_storage_config_from_db(&db)
        .await
        .expect("get storage config")
        .expect("config exists");

    assert_eq!(fetched.mode, StorageMode::ExternalNfs);
    assert_eq!(fetched.mount_path, "/data");
    assert_eq!(fetched.config_json["server"], "nas.local");
    assert!(fetched.validated());
}

#[test]
fn media_chart_values_mount_media_data_at_data() {
    let storage = managed_storage_config();
    let values = media_storage_helm_values(&storage).expect("helm values");

    assert!(values.contains(&"storage.media.existingClaim=media-data".to_string()));
    assert!(values.contains(&"storage.media.mountPath=/data".to_string()));
}

#[test]
fn media_chart_values_do_not_contain_app_specific_storage() {
    let storage = managed_storage_config();
    let values = media_storage_helm_values(&storage).expect("helm values");
    let joined = values.join("\n");

    for app_value in [
        "qbittorrent",
        "transmission",
        "sabnzbd",
        "deluge",
        "rutorrent",
        "radarr",
        "sonarr",
        "jellyfin",
        "plex",
        "rootFolder",
        "downloadDir",
        "transcode",
        "cache",
        "config/",
    ] {
        assert!(
            !joined.contains(app_value),
            "backend storage values must stay chart-agnostic: {app_value}"
        );
    }
}
