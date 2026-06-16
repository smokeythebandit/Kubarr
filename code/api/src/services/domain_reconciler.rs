use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::error::{AppError, Result};
use crate::models::prelude::*;
use crate::models::{domain, dynamic_dns_profile};
use crate::services::K8sClient;

mod cert_manager;
mod k8s;
mod transip;

pub(super) const CERT_NAMESPACE: &str = "openresty";

pub struct DomainReconciler {
    db: sea_orm::DatabaseConnection,
    k8s: Arc<tokio::sync::RwLock<Option<K8sClient>>>,
}

impl DomainReconciler {
    pub fn new(
        db: sea_orm::DatabaseConnection,
        k8s: Arc<tokio::sync::RwLock<Option<K8sClient>>>,
    ) -> Self {
        Self { db, k8s }
    }

    pub fn run(self: Arc<Self>, interval: Duration) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                if let Err(error) = self.reconcile_once().await {
                    tracing::warn!(%error, "Domain reconciliation failed");
                }
            }
        });
    }

    pub async fn reconcile_once(&self) -> Result<()> {
        self.reconcile_dynamic_dns_profiles().await?;
        self.reconcile_domain_dns_records().await?;
        self.reconcile_certificates().await?;
        Ok(())
    }

    async fn reconcile_dynamic_dns_profiles(&self) -> Result<()> {
        let profiles = DynamicDnsProfile::find()
            .filter(dynamic_dns_profile::Column::Enabled.eq(true))
            .all(&self.db)
            .await?;

        for profile in profiles {
            let result = self.check_dynamic_dns_profile(&profile).await;
            let (status, last_error) = match result {
                Ok(status) => (status, None),
                Err(error) => ("error".to_string(), Some(error.to_string())),
            };
            let mut active: dynamic_dns_profile::ActiveModel = profile.into();
            active.status = Set(status);
            active.last_error = Set(last_error);
            active.updated_at = Set(Utc::now());
            active.update(&self.db).await?;
        }

        Ok(())
    }

    async fn check_dynamic_dns_profile(
        &self,
        profile: &dynamic_dns_profile::Model,
    ) -> Result<String> {
        match profile.provider.as_str() {
            "manual" => Ok("manual".to_string()),
            "transip" => self.check_transip_profile(profile).await,
            _ => Ok("configured".to_string()),
        }
    }

    async fn reconcile_domain_dns_records(&self) -> Result<()> {
        let domains = Domain::find()
            .filter(domain::Column::Enabled.eq(true))
            .filter(domain::Column::DnsMode.eq("dynamic_dns"))
            .all(&self.db)
            .await?;
        if domains.is_empty() {
            return Ok(());
        }

        let public_ip = match discover_public_ipv4().await {
            Ok(ip) => ip,
            Err(error) => {
                tracing::warn!(%error, "Failed to discover public IPv4 for Dynamic DNS");
                return Ok(());
            }
        };

        for domain_model in domains {
            if let Err(error) = self
                .reconcile_domain_dns_record(domain_model.clone(), &public_ip)
                .await
            {
                tracing::warn!(domain = domain_model.domain, %error, "Failed to reconcile DNS record");
                self.update_domain_dns_status(domain_model, format!("error: {}", error))
                    .await?;
            }
        }

        Ok(())
    }

    async fn reconcile_domain_dns_record(
        &self,
        domain_model: domain::Model,
        public_ip: &str,
    ) -> Result<()> {
        let profile_id = domain_model.ddns_profile_id.ok_or_else(|| {
            AppError::BadRequest("Domain is missing Dynamic DNS profile".to_string())
        })?;
        let profile = DynamicDnsProfile::find_by_id(profile_id)
            .one(&self.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Dynamic DNS profile not found".to_string()))?;

        match profile.provider.as_str() {
            "manual" => {
                self.update_domain_dns_status(domain_model, "manual".to_string())
                    .await?;
            }
            "transip" => {
                self.upsert_transip_a_record(&profile, &domain_model, public_ip)
                    .await?;
                self.update_domain_dns_status(domain_model, "updated".to_string())
                    .await?;
            }
            provider => {
                self.update_domain_dns_status(
                    domain_model,
                    format!("unsupported_provider:{}", provider),
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn update_domain_dns_status(
        &self,
        domain_model: domain::Model,
        status: String,
    ) -> Result<()> {
        let mut active: domain::ActiveModel = domain_model.into();
        active.dns_status = Set(status);
        active.updated_at = Set(Utc::now());
        active.update(&self.db).await?;
        Ok(())
    }
}

async fn discover_public_ipv4() -> Result<String> {
    let value = reqwest::Client::new()
        .get("https://api.ipify.org")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?
        .trim()
        .to_string();

    if value.parse::<std::net::Ipv4Addr>().is_ok() {
        Ok(value)
    } else {
        Err(AppError::BadGateway(
            "Public IP discovery returned an invalid IPv4 address".to_string(),
        ))
    }
}
