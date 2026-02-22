use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct CreateInvite {
    #[serde(default = "default_invite_days")]
    pub expires_in_days: i32,
}

fn default_invite_days() -> i32 {
    7
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteResponse {
    pub id: i64,
    pub code: String,
    pub created_by_id: i64,
    pub created_by_username: String,
    pub used_by_id: Option<i64>,
    pub used_by_username: Option<String>,
    pub is_used: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_invite_default_days() {
        let r: CreateInvite = serde_json::from_str("{}").expect("deser");
        assert_eq!(r.expires_in_days, 7);
    }

    #[test]
    fn create_invite_custom_days() {
        let r: CreateInvite = serde_json::from_str(r#"{"expires_in_days":30}"#).expect("deser");
        assert_eq!(r.expires_in_days, 30);
    }

    #[test]
    fn invite_response_ser() {
        use chrono::Utc;
        let r = InviteResponse {
            id: 1,
            code: "ABC123".to_string(),
            created_by_id: 10,
            created_by_username: "admin".to_string(),
            used_by_id: None,
            used_by_username: None,
            is_used: false,
            expires_at: None,
            created_at: Utc::now(),
            used_at: None,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"code\":\"ABC123\""));
        assert!(json.contains("\"is_used\":false"));
    }

    #[test]
    fn invite_response_ser_used() {
        use chrono::Utc;
        let r = InviteResponse {
            id: 2,
            code: "XYZ789".to_string(),
            created_by_id: 10,
            created_by_username: "admin".to_string(),
            used_by_id: Some(5),
            used_by_username: Some("alice".to_string()),
            is_used: true,
            expires_at: Some(Utc::now()),
            created_at: Utc::now(),
            used_at: Some(Utc::now()),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"is_used\":true"));
        assert!(json.contains("\"used_by_username\":\"alice\""));
    }
}
