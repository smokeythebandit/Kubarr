use base64::prelude::{Engine, BASE64_STANDARD};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha512;

use super::DomainReconciler;
use crate::error::{AppError, Result};
use crate::models::{domain, dynamic_dns_profile};

impl DomainReconciler {
    pub(super) async fn check_transip_profile(
        &self,
        profile: &dynamic_dns_profile::Model,
    ) -> Result<String> {
        let config: Value = serde_json::from_str(&profile.config_json)?;
        let token = transip_access_token(&config).await?;
        let zone = config
            .get("zone")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::BadRequest("TransIP zone is missing".to_string()))?;

        let url = format!("https://api.transip.nl/v6/domains/{}/dns", zone.trim());
        let response = reqwest::Client::new()
            .get(url)
            .bearer_auth(token)
            .send()
            .await?;

        if response.status().is_success() {
            Ok("verified".to_string())
        } else {
            Err(AppError::BadGateway(format!(
                "TransIP DNS check failed with status {}",
                response.status()
            )))
        }
    }

    pub(super) async fn upsert_transip_a_record(
        &self,
        profile: &dynamic_dns_profile::Model,
        domain_model: &domain::Model,
        public_ip: &str,
    ) -> Result<()> {
        let (zone, mut dns_response, url, token) = self.read_transip_dns_entries(profile).await?;
        let record_name = transip_record_name(&domain_model.domain, &zone)?;

        dns_response
            .dns_entries
            .retain(|entry| !(entry.name == record_name && entry.record_type == "A"));
        dns_response.dns_entries.push(TransipDnsEntry {
            name: record_name,
            expire: 300,
            record_type: "A".to_string(),
            content: public_ip.to_string(),
        });

        self.write_transip_dns_entries(&url, &token, dns_response.dns_entries)
            .await
    }

    async fn read_transip_dns_entries(
        &self,
        profile: &dynamic_dns_profile::Model,
    ) -> Result<(String, TransipDnsResponse, String, String)> {
        let config: Value = serde_json::from_str(&profile.config_json)?;
        let token = transip_access_token(&config).await?;
        let zone = config
            .get("zone")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::BadRequest("TransIP zone is missing".to_string()))?
            .trim()
            .trim_start_matches("*.")
            .to_string();
        let url = format!("https://api.transip.nl/v6/domains/{}/dns", zone);

        let response = reqwest::Client::new()
            .get(&url)
            .bearer_auth(token.trim())
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(AppError::BadGateway(format!(
                "TransIP DNS read failed with status {}",
                response.status()
            )));
        }

        let dns_response = response.json().await?;
        Ok((zone, dns_response, url, token.trim().to_string()))
    }

    async fn write_transip_dns_entries(
        &self,
        url: &str,
        token: &str,
        entries: Vec<TransipDnsEntry>,
    ) -> Result<()> {
        let response = reqwest::Client::new()
            .put(url)
            .bearer_auth(token.trim())
            .json(&json!({ "dnsEntries": entries }))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(AppError::BadGateway(format!(
                "TransIP DNS update failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct TransipDnsResponse {
    #[serde(rename = "dnsEntries")]
    dns_entries: Vec<TransipDnsEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TransipDnsEntry {
    name: String,
    expire: i32,
    #[serde(rename = "type")]
    record_type: String,
    content: String,
}

fn transip_record_name(domain: &str, zone: &str) -> Result<String> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let zone = zone.trim().trim_end_matches('.').to_ascii_lowercase();

    if domain == zone {
        return Ok("@".to_string());
    }

    let suffix = format!(".{}", zone);
    if let Some(name) = domain.strip_suffix(&suffix) {
        return Ok(name.to_string());
    }

    Err(AppError::BadRequest(format!(
        "Domain {} is not inside TransIP zone {}",
        domain, zone
    )))
}

pub(super) fn transip_solver_secret_name(profile_id: i64) -> String {
    format!("kubarr-transip-dns-{}", profile_id)
}

pub(super) fn transip_auth_type(config: &Value) -> String {
    config
        .get("auth_type")
        .or_else(|| config.get("authType"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("token")
        .to_string()
}

pub(super) fn copy_string_config(target: &mut Value, source: &Value, key: &str) {
    if let Some(value) = source.get(key).and_then(Value::as_str) {
        target[key] = Value::String(value.to_string());
    } else if let Some(value) = source.get(key).and_then(Value::as_bool) {
        target[key] = Value::String(value.to_string());
    }
}

async fn transip_access_token(config: &Value) -> Result<String> {
    if transip_auth_type(config) == "private_key" {
        return transip_access_token_from_private_key(config).await;
    }

    config
        .get("token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .ok_or_else(|| AppError::BadRequest("TransIP token is missing".to_string()))
}

async fn transip_access_token_from_private_key(config: &Value) -> Result<String> {
    let login = config
        .get("login")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("TransIP login is missing".to_string()))?;
    let private_key_pem = config
        .get("private_key")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::BadRequest("TransIP private key is missing".to_string()))?;
    let global_key = config
        .get("global_key")
        .or_else(|| config.get("globalKey"))
        .and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_str().map(|text| text == "true"))
        })
        .unwrap_or(true);

    let body = json!({
        "login": login.trim(),
        "nonce": uuid::Uuid::new_v4().simple().to_string(),
        "read_only": false,
        "expiration_time": "30 minutes",
        "label": "kubarr",
        "global_key": global_key,
    });
    let body_bytes = serde_json::to_vec(&body)?;
    let key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(private_key_pem))
        .map_err(|error| AppError::BadRequest(format!("Invalid TransIP private key: {}", error)))?;
    let signing_key = SigningKey::<Sha512>::new(key);
    let signature = signing_key.sign_with_rng(&mut rsa::rand_core::OsRng, &body_bytes);
    let signature = BASE64_STANDARD.encode(signature.to_bytes());

    let response = reqwest::Client::new()
        .post("https://api.transip.nl/v6/auth")
        .header("Content-Type", "application/json")
        .header("Signature", signature)
        .body(body_bytes)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(AppError::BadGateway(format!(
            "TransIP auth failed with status {}",
            response.status()
        )));
    }
    let body: Value = response.json().await?;
    body.get("token")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .ok_or_else(|| {
            AppError::BadGateway("TransIP auth response did not include a token".to_string())
        })
}
