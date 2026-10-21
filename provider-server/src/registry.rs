//! Provider registration and heartbeat with the subscribe-server registry.

use crate::config::{AdvertiseAddress, Config};
use crate::manifest::NodeManifest;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Serialize)]
struct RegistryRegisterRequest {
    provider_id: String,
    name: String,
    version: String,
    addresses: Vec<AdvertiseAddress>,
    node_links: Vec<String>,
    heartbeat_interval_secs: u64,
}

#[derive(Debug, Serialize)]
struct RegistryHeartbeatRequest {
    provider_id: String,
    heartbeat_token: String,
    version: String,
    addresses: Vec<AdvertiseAddress>,
    node_links: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryRegisterResponse {
    heartbeat_token: String,
    heartbeat_interval_secs: u64,
}

/// Outcome of a single registry interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryStep {
    Registered {
        heartbeat_token: String,
        interval_secs: u64,
    },
    Heartbeat,
    Rejected,
    RequestFailed,
}

/// Long-running registry client. [`RegistryReporter::step`] performs exactly one
/// HTTP interaction so the register -> heartbeat -> re-register state machine is
/// directly testable.
pub struct RegistryReporter {
    client: reqwest::Client,
    registry_url: String,
    registry_token: String,
    provider_id: String,
    name: String,
    advertise_addresses: Vec<AdvertiseAddress>,
    node_links: Vec<String>,
    heartbeat_token: Option<String>,
    interval_secs: u64,
}

impl RegistryReporter {
    /// Build a reporter from configuration; `Ok(None)` means registration is off.
    pub fn new(config: &Config) -> Result<Option<Self>> {
        if config.registry_url.trim().is_empty() {
            return Ok(None);
        }
        let registry_token = config.registry_token.trim().to_owned();
        if registry_token.is_empty() {
            anyhow::bail!("registry_url is configured but registry_token is empty");
        }
        let provider_id = if config.registry_provider_id.trim().is_empty() {
            format!("provider-{}", uuid::Uuid::new_v4())
        } else {
            config.registry_provider_id.trim().to_owned()
        };
        let name = if config.registry_name.trim().is_empty() {
            provider_id.clone()
        } else {
            config.registry_name.trim().to_owned()
        };
        let default_manifest = NodeManifest::from_config(config);
        let node_links = if config.node_links.is_empty() {
            default_manifest.link_lines()
        } else {
            config.node_links.clone()
        };
        let advertise_addresses = if config.advertise_addresses.is_empty() {
            vec![AdvertiseAddress {
                kind: "local".into(),
                value: default_manifest.node_ip,
            }]
        } else {
            config.advertise_addresses.clone()
        };
        let client = reqwest::Client::builder()
            .user_agent(format!(
                "vpn233-provider-server/{}",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout(Duration::from_secs(15))
            .build()
            .context("cannot build registry HTTP client")?;
        Ok(Some(Self {
            client,
            registry_url: config.registry_url.trim().trim_end_matches('/').to_owned(),
            registry_token,
            provider_id,
            name,
            advertise_addresses,
            node_links,
            heartbeat_token: None,
            interval_secs: config.heartbeat_interval_secs.clamp(10, 3600),
        }))
    }

    pub fn heartbeat_token(&self) -> Option<&str> {
        self.heartbeat_token.as_deref()
    }

    pub fn interval_secs(&self) -> u64 {
        self.interval_secs
    }

    /// Run one registry interaction, updating the internal heartbeat token.
    pub async fn step(&mut self) -> Result<RegistryStep> {
        if let Some(token) = self.heartbeat_token.clone() {
            return self.send_heartbeat(token).await;
        }
        self.send_register().await
    }

    async fn send_heartbeat(&mut self, token: String) -> Result<RegistryStep> {
        let url = format!("{}/api/v1/registry/heartbeat", self.registry_url);
        let request = RegistryHeartbeatRequest {
            provider_id: self.provider_id.clone(),
            heartbeat_token: token,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            addresses: self.advertise_addresses.clone(),
            node_links: self.node_links.clone(),
        };
        match self
            .client
            .post(&url)
            .header("x-provider-registry-token", &self.registry_token)
            .json(&request)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => Ok(RegistryStep::Heartbeat),
            Ok(response) => {
                warn!(
                    status = %response.status(),
                    "provider registry heartbeat rejected; registering again"
                );
                self.heartbeat_token = None;
                Ok(RegistryStep::Rejected)
            }
            Err(_) => {
                warn!("provider registry heartbeat request failed");
                Ok(RegistryStep::RequestFailed)
            }
        }
    }

    async fn send_register(&mut self) -> Result<RegistryStep> {
        let url = format!("{}/api/v1/registry/register", self.registry_url);
        let request = RegistryRegisterRequest {
            provider_id: self.provider_id.clone(),
            name: self.name.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            addresses: self.advertise_addresses.clone(),
            node_links: self.node_links.clone(),
            heartbeat_interval_secs: self.interval_secs,
        };
        let response = self
            .client
            .post(&url)
            .header("x-provider-registry-token", &self.registry_token)
            .json(&request)
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("provider registry registration request failed")?;
        if !response.status().is_success() {
            warn!(status = %response.status(), "provider registry registration rejected");
            return Ok(RegistryStep::Rejected);
        }
        let payload: RegistryRegisterResponse = response
            .json()
            .await
            .map_err(reqwest::Error::without_url)
            .context("invalid provider registry registration response")?;
        self.heartbeat_token = Some(payload.heartbeat_token.clone());
        self.interval_secs = payload.heartbeat_interval_secs.clamp(10, 3600);
        info!("provider registered with the subscribe-server");
        Ok(RegistryStep::Registered {
            heartbeat_token: payload.heartbeat_token,
            interval_secs: self.interval_secs,
        })
    }

    /// Keep registering/heartbeating forever. Failures are logged, never fatal.
    pub async fn run(mut self) {
        loop {
            if let Err(error) = self.step().await {
                warn!(%error, "provider registry step failed");
            }
            tokio::time::sleep(Duration::from_secs(self.interval_secs)).await;
        }
    }
}
