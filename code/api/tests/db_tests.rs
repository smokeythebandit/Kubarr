mod common;

use common::{
    create_test_db, create_test_db_with_seed, create_test_user, create_test_user_with_role,
};

#[tokio::test]
async fn test_create_test_db() {
    let db = create_test_db().await;
    assert!(db.ping().await.is_ok());
}

#[tokio::test]
async fn test_create_test_db_with_seed() {
    use kubarr::models::prelude::*;
    use sea_orm::EntityTrait;

    let db = create_test_db_with_seed().await;

    // Verify roles were created
    let roles = Role::find().all(&db).await.unwrap();
    assert_eq!(roles.len(), 3);

    let role_names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
    assert!(role_names.contains(&"admin"));
    assert!(role_names.contains(&"viewer"));
    assert!(role_names.contains(&"downloader"));
}

#[tokio::test]
async fn test_create_test_user() {
    let db = create_test_db().await;

    let user = create_test_user(&db, "testuser", "test@example.com", "password123", true).await;

    assert_eq!(user.username, "testuser");
    assert_eq!(user.email, "test@example.com");
    assert!(user.is_active);
    assert!(user.is_approved);
}

#[tokio::test]
async fn test_create_test_user_with_role() {
    use kubarr::models::prelude::*;
    use kubarr::models::user_role;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = create_test_db_with_seed().await;

    let user = create_test_user_with_role(
        &db,
        "admin_user",
        "admin@example.com",
        "password123",
        "admin",
    )
    .await;

    // Verify user has admin role
    let user_roles = UserRole::find()
        .filter(user_role::Column::UserId.eq(user.id))
        .all(&db)
        .await
        .unwrap();

    assert_eq!(user_roles.len(), 1);
}
