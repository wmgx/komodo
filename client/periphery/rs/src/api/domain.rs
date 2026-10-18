//! Versioned, path-free RPC for administrator configured dedicated Traefik entrances.
use mogh_resolver::Resolve;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainRoute {
  pub hostname: String,
  pub backend_url: String,
  pub entrypoints: Vec<String>,
  pub require_https: bool,
  pub certificate_resolver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainIngressCapabilities {
  pub rpc_version: u32,
  pub enabled: bool,
  pub dedicated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainRouteObservation {
  pub exists: bool,
  pub fingerprint: Option<String>,
  pub fencing_token: u64,
  /// File publication is not evidence that Traefik loaded it.
  pub load_verified: bool,
  #[serde(default)]
  pub load_error: Option<String>,
  #[serde(default)]
  pub rolled_back: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[response(DomainIngressCapabilities)]
#[error(anyhow::Error)]
pub struct GetDomainIngressCapabilities {
  pub ingress_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[response(DomainRouteObservation)]
#[error(anyhow::Error)]
pub struct ObserveDomainRoute {
  pub ingress_id: String,
  pub domain_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[response(DomainRouteObservation)]
#[error(anyhow::Error)]
pub struct ApplyDomainRoute {
  pub ingress_id: String,
  pub domain_id: String,
  pub fencing_token: u64,
  /// Unix milliseconds; rejected once elapsed or more than 60s in the future.
  pub expires_at_ms: u64,
  pub expected_fingerprint: Option<String>,
  pub route: DomainRoute,
}

#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[response(DomainRouteObservation)]
#[error(anyhow::Error)]
pub struct RemoveDomainRoute {
  pub ingress_id: String,
  pub domain_id: String,
  pub fencing_token: u64,
  /// Unix milliseconds; rejected once elapsed or more than 60s in the future.
  pub expires_at_ms: u64,
  pub expected_fingerprint: Option<String>,
}
