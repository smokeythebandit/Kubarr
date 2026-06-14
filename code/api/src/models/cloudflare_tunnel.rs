use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "cloudflare_tunnels")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    /// Tunnel token (from CF API or user-provided) — never serialised in API responses
    #[serde(skip_serializing)]
    pub tunnel_token: String,
    /// Deployment status: not_deployed | deploying | running | failed | removing
    pub status: String,
    pub error: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    // ── Cloudflare API fields (populated by the guided wizard) ───────────────
    /// CF API token — never serialised in API responses
    #[serde(skip_serializing)]
    pub api_token: Option<String>,
    /// CF account ID
    pub account_id: Option<String>,
    /// CF tunnel UUID
    pub tunnel_id: Option<String>,
    /// CF zone ID
    pub zone_id: Option<String>,
    /// Zone name, e.g. "example.com"
    pub zone_name: Option<String>,
    /// Subdomain entered by the user, e.g. "kubarr"
    pub subdomain: Option<String>,
    /// CF DNS record ID (used for cleanup)
    pub dns_record_id: Option<String>,
    /// Full hostname, e.g. "kubarr.example.com"
    pub hostname: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_model() -> Model {
        Model {
            id: 1,
            name: "my-tunnel".to_string(),
            tunnel_token: "super-secret-token".to_string(),
            status: "running".to_string(),
            error: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            api_token: Some("cf-api-secret".to_string()),
            account_id: Some("acc123".to_string()),
            tunnel_id: Some("tid-uuid".to_string()),
            zone_id: Some("zone456".to_string()),
            zone_name: Some("example.com".to_string()),
            subdomain: Some("kubarr".to_string()),
            dns_record_id: Some("dns-rec-id".to_string()),
            hostname: Some("kubarr.example.com".to_string()),
        }
    }

    /// tunnel_token must never appear in serialised JSON — it is a secret.
    #[test]
    fn tunnel_token_is_not_serialised() {
        let model = make_model();
        let json = serde_json::to_string(&model).expect("serialise");
        assert!(
            !json.contains("super-secret-token"),
            "tunnel_token must not be present in JSON output: {json}"
        );
        assert!(
            !json.contains("tunnel_token"),
            "tunnel_token key must not be present in JSON output: {json}"
        );
    }

    /// api_token must never appear in serialised JSON — it is a secret.
    #[test]
    fn api_token_is_not_serialised() {
        let model = make_model();
        let json = serde_json::to_string(&model).expect("serialise");
        assert!(
            !json.contains("cf-api-secret"),
            "api_token value must not be present in JSON output: {json}"
        );
        assert!(
            !json.contains("api_token"),
            "api_token key must not be present in JSON output: {json}"
        );
    }

    /// Non-sensitive fields must still be present in serialised JSON.
    #[test]
    fn public_fields_are_serialised() {
        let model = make_model();
        let json = serde_json::to_string(&model).expect("serialise");
        assert!(
            json.contains("\"name\":\"my-tunnel\""),
            "name missing: {json}"
        );
        assert!(
            json.contains("\"status\":\"running\""),
            "status missing: {json}"
        );
        assert!(
            json.contains("\"hostname\":\"kubarr.example.com\""),
            "hostname missing: {json}"
        );
        assert!(
            json.contains("\"zone_name\":\"example.com\""),
            "zone_name missing: {json}"
        );
    }

    /// Verify all recognised status values are plain strings (no enum conversion errors).
    #[test]
    fn status_values_roundtrip_via_json() {
        let statuses = ["not_deployed", "deploying", "running", "failed", "removing"];
        for status in statuses {
            let mut model = make_model();
            model.status = status.to_string();
            let json = serde_json::to_string(&model).expect("serialise");
            let back: serde_json::Value = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(
                back["status"].as_str().unwrap(),
                status,
                "status roundtrip failed for: {status}"
            );
        }
    }
}
