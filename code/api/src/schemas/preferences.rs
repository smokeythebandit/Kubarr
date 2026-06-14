use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct UserPreferencesResponse {
    pub theme: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserPreferences {
    pub theme: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_preferences_response_ser() {
        let r = UserPreferencesResponse {
            theme: "dark".to_string(),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"theme\":\"dark\""));
    }

    #[test]
    fn update_user_preferences_deser_some() {
        let u: UpdateUserPreferences = serde_json::from_str(r#"{"theme":"light"}"#).expect("deser");
        assert_eq!(u.theme.as_deref(), Some("light"));
    }

    #[test]
    fn update_user_preferences_deser_none() {
        let u: UpdateUserPreferences = serde_json::from_str("{}").expect("deser");
        assert!(u.theme.is_none());
    }
}
