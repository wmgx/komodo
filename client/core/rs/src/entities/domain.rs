//! Domain management public contract. Secrets and provider object ownership are never embedded in desired state.
use super::{
  I64,
  resource::{AddFilters, Resource, ResourceQuery},
};
use partial_derive2::Partial;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

#[typeshare]
pub type Domain = Resource<DomainConfig, DomainInfo>;
#[typeshare]
pub type DnsProvider = Resource<DnsProviderConfig, ConnectionInfo>;
#[typeshare]
pub type Ingress = Resource<IngressConfig, ConnectionInfo>;
#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum TargetScheme {
  #[default]
  Http,
  Https,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "params", deny_unknown_fields)]
pub enum DomainTarget {
  Deployment {
    id: String,
    port: u16,
    scheme: TargetScheme,
  },
  StackService {
    stack_id: String,
    service: String,
    port: u16,
    scheme: TargetScheme,
  },
  Url {
    url: String,
  },
}

impl Default for DomainTarget {
  fn default() -> Self {
    Self::Url { url: String::new() }
  }
}
#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum ProxyMode {
  #[default]
  DnsOnly,
  Proxied,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "params", deny_unknown_fields)]
pub enum DomainDns {
  #[default]
  External,
  Managed {
    provider: String,
    ttl: u32,
    proxy_mode: ProxyMode,
  },
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum TlsMode {
  #[default]
  RequireHttps,
  HttpOnly,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DomainTls {
  pub mode: TlsMode,
  #[serde(default)]
  pub profile: String,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum ReconcileMode {
  #[default]
  Manual,
  Automatic,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Partial, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[partial_derive(Debug, Clone, Default, Serialize, Deserialize)]
#[diff_derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(
  feature = "schemars",
  partial_derive(schemars::JsonSchema)
)]
#[partial(skip_serializing_none, from, diff)]
#[serde(deny_unknown_fields)]
pub struct DomainConfig {
  pub hostname: String,
  pub target: DomainTarget,
  #[serde(default)]
  pub dns: DomainDns,
  pub ingress: String,
  #[serde(default)]
  pub tls: DomainTls,
  #[serde(default)]
  pub reconcile_mode: ReconcileMode,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum DomainState {
  #[default]
  Pending,
  Applying,
  Ready,
  Degraded,
  Blocked,
  Deleting,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum ConditionStatus {
  True,
  False,
  #[default]
  Unknown,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum DomainConditionType {
  TargetResolved,
  IngressReady,
  DnsConfigured,
  TlsReady,
  Reachable,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainCondition {
  pub condition: DomainConditionType,
  pub status: ConditionStatus,
  pub reason: String,
  pub message: String,
  pub observed_at: I64,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainInfo {
  /// Original user-facing hostname. Normalized ASCII config.hostname remains the identity key.
  #[serde(default)]
  pub display_hostname: String,
  pub generation: I64,
  pub observed_generation: I64,
  pub config_digest: String,
  pub last_operation_id: Option<String>,
  pub state: DomainState,
  #[serde(default)]
  pub conditions: Vec<DomainCondition>,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ConnectionInfo {
  pub generation: I64,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum SecretSource {
  #[default]
  CoreSecret,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
  pub source: SecretSource,
  pub name: String,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Partial, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[partial_derive(Debug, Clone, Default, Serialize, Deserialize)]
#[diff_derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(
  feature = "schemars",
  partial_derive(schemars::JsonSchema)
)]
#[partial(skip_serializing_none, from, diff)]
#[serde(deny_unknown_fields)]
pub struct DnsProviderConfig {
  pub adapter: String,
  pub schema_version: u32,
  pub account_id: String,
  pub zone_id: String,
  pub secret: SecretRef,
  #[serde(default)]
  pub allowed_hostname_suffixes: Vec<String>,
}

#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TlsProfile {
  pub name: String,
  pub mode: TlsMode,
  #[serde(default)]
  pub certificate_resolver: String,
  #[serde(default)]
  pub requires_dns: bool,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "params", deny_unknown_fields)]
pub enum IngressAdapterConfig {
  TraefikFileV1 {
    server: String,
    entrypoint: String,
    dns_target: String,
    dedicated: bool,
  },
  CloudflareTunnelV1 {
    account_id: String,
    tunnel_id: String,
    secret: SecretRef,
    dedicated: bool,
  },
}

impl Default for IngressAdapterConfig {
  fn default() -> Self {
    Self::TraefikFileV1 {
      server: String::new(),
      entrypoint: "websecure".into(),
      dns_target: String::new(),
      dedicated: false,
    }
  }
}
#[typeshare]
#[derive(
  Debug, Clone, PartialEq, Default, Partial, Serialize, Deserialize,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[partial_derive(Debug, Clone, Default, Serialize, Deserialize)]
#[diff_derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(
  feature = "schemars",
  partial_derive(schemars::JsonSchema)
)]
#[partial(skip_serializing_none, from, diff)]
#[serde(deny_unknown_fields)]
pub struct IngressConfig {
  pub schema_version: u32,
  pub adapter: IngressAdapterConfig,
  #[serde(default)]
  pub allowed_hostname_suffixes: Vec<String>,
  #[serde(default)]
  pub allowed_networks: Vec<String>,
  #[serde(default)]
  pub docker_network: String,
  #[serde(default)]
  pub tls_profiles: Vec<TlsProfile>,
  #[serde(default)]
  pub drain_seconds: u32,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainAdapterCapabilities {
  pub adapter: String,
  pub schema_version: u32,
  pub target_types: Vec<String>,
  pub tls_modes: Vec<TlsMode>,
  pub dns_types: Vec<String>,
  pub proxy_modes: Vec<ProxyMode>,
  pub migration_modes: Vec<String>,
  pub tls_bootstrap_requires_dns: bool,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainError {
  pub code: String,
  pub message: String,
  pub retryable: bool,
  pub field: Option<String>,
  pub update_id: Option<String>,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "params", deny_unknown_fields)]
pub enum DomainChangeIntent {
  Create {
    name: String,
    config: DomainConfig,
  },
  Update {
    domain: String,
    config: DomainConfig,
    expected_generation: I64,
  },
  Reconcile {
    domain: String,
  },
  Remove {
    domain: String,
  },
  Migrate {
    domain: String,
    dns_provider: Option<String>,
    ingress: Option<String>,
  },
  Adopt {
    domain: String,
    objects: Vec<String>,
  },
  Abandon {
    domain: String,
  },
  FinalizeMigration {
    domain: String,
  },
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum DomainPlanOperation {
  Create,
  Update,
  Reconcile,
  Remove,
  Migrate,
  Adopt,
  Abandon,
  FinalizeMigration,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainPlan {
  pub plan_id: String,
  pub domain_id: Option<String>,
  pub name: String,
  pub generation: I64,
  pub expires_at: I64,
  pub operation: DomainPlanOperation,
  pub steps: Vec<String>,
  pub warnings: Vec<String>,
  pub requires_downtime: bool,
  pub request_id: String,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainStatus {
  pub domain_id: String,
  pub generation: I64,
  pub observed_generation: I64,
  pub state: DomainState,
  pub conditions: Vec<DomainCondition>,
  pub update_id: Option<String>,
  pub error: Option<DomainError>,
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "params", deny_unknown_fields)]
pub enum DomainConnection {
  DnsProvider(String),
  Ingress(String),
}

#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainConnectionValidation {
  pub valid: bool,
  pub capabilities: DomainAdapterCapabilities,
  pub errors: Vec<DomainError>,
}

#[typeshare(serialized_as = "Partial<DomainConfig>")]
pub type _PartialDomainConfig = PartialDomainConfig;
#[cfg(feature = "utoipa")]
impl utoipa::PartialSchema for PartialDomainConfig {
  fn schema()
  -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    utoipa::schema!(#[inline] std::collections::HashMap<String,serde_json::Value>).into()
  }
}
#[cfg(feature = "utoipa")]
impl utoipa::ToSchema for PartialDomainConfig {}
#[typeshare]
pub type DomainQuery = ResourceQuery<DomainQuerySpecifics>;

#[typeshare(serialized_as = "Partial<DnsProviderConfig>")]
pub type _PartialDnsProviderConfig = PartialDnsProviderConfig;
#[cfg(feature = "utoipa")]
impl utoipa::PartialSchema for PartialDnsProviderConfig {
  fn schema()
  -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    utoipa::schema!(#[inline] std::collections::HashMap<String,serde_json::Value>).into()
  }
}
#[cfg(feature = "utoipa")]
impl utoipa::ToSchema for PartialDnsProviderConfig {}
#[typeshare]
pub type DnsProviderQuery = ResourceQuery<DomainQuerySpecifics>;

#[typeshare(serialized_as = "Partial<IngressConfig>")]
pub type _PartialIngressConfig = PartialIngressConfig;
#[cfg(feature = "utoipa")]
impl utoipa::PartialSchema for PartialIngressConfig {
  fn schema()
  -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    utoipa::schema!(#[inline] std::collections::HashMap<String,serde_json::Value>).into()
  }
}
#[cfg(feature = "utoipa")]
impl utoipa::ToSchema for PartialIngressConfig {}
#[typeshare]
pub type IngressQuery = ResourceQuery<DomainQuerySpecifics>;

#[typeshare]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
pub struct DomainQuerySpecifics {}
impl AddFilters for DomainQuerySpecifics {}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn safe_defaults_and_plaintext_rejection() {
    let config: DomainConfig = serde_json::from_value(serde_json::json!({
      "hostname":"app.example.com", "target":{"type":"Url","params":{"url":"https://app.example.com"}}, "ingress":"edge"
    })).unwrap();
    assert_eq!(config.tls.mode, TlsMode::RequireHttps);
    assert_eq!(config.reconcile_mode, ReconcileMode::Manual);
    assert_eq!(config.dns, DomainDns::External);
    let mut json = serde_json::to_value(config).unwrap();
    json["cloudflare_token"] = "do-not-accept".into();
    assert!(serde_json::from_value::<DomainConfig>(json).is_err());
  }
  // Snapshot of the pre-domain resource-target wire variants. Old clients can
  // still read existing resources, but must upgrade before reading new types.
  #[test]
  fn legacy_client_enum_cannot_decode_new_domain_targets() {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    #[serde(tag = "type", content = "id")]
    enum LegacyTarget {
      System(String),
      Swarm(String),
      Server(String),
      Stack(String),
      Deployment(String),
      Build(String),
      Repo(String),
      Procedure(String),
      Action(String),
      Builder(String),
      Alerter(String),
      ResourceSync(String),
    }
    assert!(
      serde_json::from_value::<LegacyTarget>(
        serde_json::json!({"type":"Deployment","id":"old"})
      )
      .is_ok()
    );
    let domain =
      crate::entities::ResourceTarget::Domain("new".into());
    assert!(
      serde_json::from_value::<LegacyTarget>(
        serde_json::to_value(domain).unwrap()
      )
      .is_err()
    );
  }
  #[test]
  fn existing_resource_wire_format_is_unchanged() {
    use crate::entities::ResourceTarget;
    let wire =
      serde_json::json!({"type":"Deployment","id":"existing"});
    let resource: ResourceTarget =
      serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(resource).unwrap(), wire);
  }
  #[test]
  fn plans_contain_references_not_configuration_or_credentials() {
    let plan = DomainPlan {
      plan_id: "server-issued".into(),
      domain_id: Some("binding".into()),
      name: "app".into(),
      generation: 1,
      expires_at: 100,
      operation: DomainPlanOperation::Reconcile,
      steps: vec![],
      warnings: vec![],
      requires_downtime: false,
      request_id: "ci-attempt".into(),
    };
    let wire = serde_json::to_value(plan).unwrap();
    assert!(wire.get("config").is_none());
    assert!(wire.get("secret").is_none());
    assert_eq!(wire["request_id"], "ci-attempt");
  }
  #[test]
  fn invalid_observation_urls_are_still_redacted_in_config_diagnostics()
   {
    let mut config =
      crate::entities::config::periphery::PeripheryConfig::default();
    config.domain_ingress_observation_urls.insert("edge".into(),"http://user:secret-sentinel@localhost:8080?token=secret-sentinel".into());
    assert!(
      !format!("{:?}", config.sanitized())
        .contains("secret-sentinel")
    );
  }
  #[test]
  fn explicit_attach_domain_is_not_implicit() {
    use crate::entities::permission::PermissionLevel;
    assert!(!crate::entities::permission::PermissionLevelAndSpecifics::from(PermissionLevel::Write).fulfills(&PermissionLevel::Read.attach_domain()));
  }
}
