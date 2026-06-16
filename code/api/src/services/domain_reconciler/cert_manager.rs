use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::{json, Value};

use super::k8s::{apply_json, cert_manager_available};
use super::transip::{copy_string_config, transip_auth_type, transip_solver_secret_name};
use super::{DomainReconciler, CERT_NAMESPACE};
use crate::error::{AppError, Result};
use crate::models::prelude::*;
use crate::models::{domain, letsencrypt_profile};

impl DomainReconciler {
    pub(super) async fn reconcile_certificates(&self) -> Result<()> {
        let domains = Domain::find()
            .filter(domain::Column::Enabled.eq(true))
            .filter(domain::Column::TlsMode.eq("letsencrypt"))
            .all(&self.db)
            .await?;

        for domain_model in domains {
            if let Err(error) = self
                .reconcile_domain_certificate(domain_model.clone())
                .await
            {
                tracing::warn!(domain = domain_model.domain, %error, "Failed to reconcile certificate");
                self.mark_domain_certificate_error(domain_model, error.to_string())
                    .await?;
            }
        }

        Ok(())
    }

    async fn reconcile_domain_certificate(&self, domain_model: domain::Model) -> Result<()> {
        let profile_id = domain_model.letsencrypt_profile_id.ok_or_else(|| {
            AppError::BadRequest("Domain is missing Let's Encrypt profile".to_string())
        })?;
        let profile = LetsEncryptProfile::find_by_id(profile_id)
            .one(&self.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Let's Encrypt profile not found".to_string()))?;

        if !profile.enabled {
            self.update_domain_certificate_status(domain_model, "profile_disabled", None)
                .await?;
            return Ok(());
        }

        let client = {
            let guard = self.k8s.read().await;
            guard
                .as_ref()
                .ok_or_else(|| {
                    AppError::ServiceUnavailable("Kubernetes not available".to_string())
                })?
                .client()
                .clone()
        };

        if !cert_manager_available(&client).await {
            self.update_domain_certificate_status(domain_model, "cert_manager_missing", None)
                .await?;
            return Ok(());
        }

        let issuer_name = issuer_name(&profile);
        let secret_name = certificate_secret_name(&domain_model.domain);
        let certificate_name = certificate_name(&domain_model.domain);

        if profile.challenge_type == "dns01" {
            self.ensure_dns01_solver_secret(&client, &profile).await?;
        }

        let issuer = self.cluster_issuer_manifest(&profile, &issuer_name).await?;
        apply_json(
            &client,
            &format!("/apis/cert-manager.io/v1/clusterissuers/{}", issuer_name),
            issuer,
        )
        .await?;

        let certificate = certificate_manifest(
            &domain_model.domain,
            &certificate_name,
            &secret_name,
            &issuer_name,
        );
        apply_json(
            &client,
            &format!(
                "/apis/cert-manager.io/v1/namespaces/{}/certificates/{}",
                CERT_NAMESPACE, certificate_name
            ),
            certificate,
        )
        .await?;

        self.update_domain_certificate_status(domain_model, "reconciled", Some(secret_name))
            .await
    }

    async fn cluster_issuer_manifest(
        &self,
        profile: &letsencrypt_profile::Model,
        name: &str,
    ) -> Result<Value> {
        let server = match profile.environment.as_str() {
            "production" => "https://acme-v02.api.letsencrypt.org/directory",
            _ => "https://acme-staging-v02.api.letsencrypt.org/directory",
        };

        let solver = if profile.challenge_type == "dns01" {
            let dns_profile_id = profile.dns_profile_id.ok_or_else(|| {
                AppError::BadRequest("DNS-01 profile is missing DNS provider".to_string())
            })?;
            let dns_profile = DynamicDnsProfile::find_by_id(dns_profile_id)
                .one(&self.db)
                .await?
                .ok_or_else(|| AppError::NotFound("Dynamic DNS profile not found".to_string()))?;

            let config: Value = serde_json::from_str(&dns_profile.config_json)?;
            let zone = config
                .get("zone")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| AppError::BadRequest("TransIP zone is missing".to_string()))?
                .trim()
                .trim_start_matches("*.")
                .to_string();

            json!({
                "dns01": {
                    "webhook": {
                        "groupName": "dns.kubarr.local",
                        "solverName": "transip",
                        "config": {
                            "zone": zone,
                            "secretName": transip_solver_secret_name(dns_profile.id),
                            "secretNamespace": CERT_NAMESPACE,
                            "tokenKey": "token"
                        }
                    }
                }
            })
        } else {
            json!({
                "http01": {
                    "ingress": {
                        "class": "kubarr-openresty"
                    }
                }
            })
        };

        Ok(json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "ClusterIssuer",
            "metadata": {
                "name": name,
                "labels": {
                    "app.kubernetes.io/managed-by": "kubarr"
                }
            },
            "spec": {
                "acme": {
                    "email": profile.email,
                    "server": server,
                    "privateKeySecretRef": {
                        "name": format!("{}-account-key", name)
                    },
                    "solvers": [solver]
                }
            }
        }))
    }

    async fn ensure_dns01_solver_secret(
        &self,
        client: &kube::Client,
        profile: &letsencrypt_profile::Model,
    ) -> Result<()> {
        let dns_profile_id = profile.dns_profile_id.ok_or_else(|| {
            AppError::BadRequest("DNS-01 profile is missing DNS provider".to_string())
        })?;
        let dns_profile = DynamicDnsProfile::find_by_id(dns_profile_id)
            .one(&self.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Dynamic DNS profile not found".to_string()))?;
        if dns_profile.provider != "transip" {
            return Err(AppError::BadRequest(format!(
                "DNS-01 provider {} is not supported by Kubarr cert-manager webhook",
                dns_profile.provider
            )));
        }

        let config: Value = serde_json::from_str(&dns_profile.config_json)?;
        let mut string_data = json!({
            "authType": transip_auth_type(&config),
        });
        copy_string_config(&mut string_data, &config, "token");
        copy_string_config(&mut string_data, &config, "login");
        copy_string_config(&mut string_data, &config, "private_key");
        copy_string_config(&mut string_data, &config, "global_key");

        self.apply_opaque_secret(
            client,
            CERT_NAMESPACE,
            &transip_solver_secret_name(dns_profile.id),
            string_data,
        )
        .await
    }

    async fn update_domain_certificate_status(
        &self,
        domain_model: domain::Model,
        status: &str,
        secret_name: Option<String>,
    ) -> Result<()> {
        let mut active: domain::ActiveModel = domain_model.into();
        active.certificate_status = Set(status.to_string());
        if secret_name.is_some() {
            active.tls_secret_name = Set(secret_name);
        }
        active.updated_at = Set(Utc::now());
        active.update(&self.db).await?;
        Ok(())
    }

    async fn mark_domain_certificate_error(
        &self,
        domain_model: domain::Model,
        error: String,
    ) -> Result<()> {
        let mut active: domain::ActiveModel = domain_model.into();
        active.certificate_status = Set(format!("error: {}", error));
        active.updated_at = Set(Utc::now());
        active.update(&self.db).await?;
        Ok(())
    }
}

fn certificate_manifest(domain: &str, name: &str, secret_name: &str, issuer_name: &str) -> Value {
    json!({
        "apiVersion": "cert-manager.io/v1",
        "kind": "Certificate",
        "metadata": {
            "name": name,
            "namespace": CERT_NAMESPACE,
            "labels": {
                "app.kubernetes.io/managed-by": "kubarr"
            }
        },
        "spec": {
            "secretName": secret_name,
            "issuerRef": {
                "name": issuer_name,
                "kind": "ClusterIssuer"
            },
            "dnsNames": [domain]
        }
    })
}

fn issuer_name(profile: &letsencrypt_profile::Model) -> String {
    format!("kubarr-le-{}", profile.id)
}

fn certificate_name(domain: &str) -> String {
    format!("kubarr-cert-{}", dns_label(domain))
}

fn certificate_secret_name(domain: &str) -> String {
    format!("kubarr-tls-{}", dns_label(domain))
}

fn dns_label(domain: &str) -> String {
    let mut label = domain
        .trim_start_matches("*.")
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if label.len() > 48 {
        label.truncate(48);
        label = label.trim_matches('-').to_string();
    }
    if domain.starts_with("*.") {
        format!("wildcard-{}", label)
    } else {
        label
    }
}
