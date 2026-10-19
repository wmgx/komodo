use bson::Document;
use derive_default_builder::DefaultBuilder;
use partial_derive2::Partial;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use super::{
  I64, JsonValue, ResourceTarget,
  resource::{AddFilters, Resource, ResourceListItem, ResourceQuery},
};

#[typeshare]
pub type Domain = Resource<DomainConfig, DomainInfo>;

#[typeshare]
pub type DomainListItem = ResourceListItem<DomainListItemInfo>;

#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DomainListItemInfo {
  /// The full hostname, eg. `app.example.com`
  pub hostname: String,
  /// Name of the account managing DNS. Empty if DNS is not managed.
  pub dns_account: String,
  /// Name of the account managing the tunnel / ingress rule.
  /// Empty if ingress is not managed.
  pub ingress_account: String,
  /// Bound Deployment / Stack, if any.
  #[serde(default)]
  pub target: Option<ResourceTarget>,
  /// Stack service name, empty otherwise.
  #[serde(default)]
  pub service: String,
  #[serde(default)]
  pub port: u16,
  /// `ready` | `drifted` | `error` | `unchecked`.
  /// Cached from the last plan / apply, listing never calls the provider.
  pub status_code: String,
}

#[typeshare(serialized_as = "Partial<DomainConfig>")]
pub type _PartialDomainConfig = PartialDomainConfig;

/// Declares how a public hostname reaches a service:
/// one DNS record and / or one tunnel ingress rule.
///
/// DNS zone is auto-detected from the account (longest suffix match).
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default, Partial)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[partial_derive(Serialize, Deserialize, Debug, Clone, Default)]
#[cfg_attr(
  feature = "schemars",
  partial_derive(schemars::JsonSchema)
)]
#[diff_derive(Serialize, Deserialize, Debug, Clone, Default)]
#[partial(skip_serializing_none, from, diff)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainConfig {
  /// The full hostname, eg. `app.example.com`. Lowercased on save.
  #[serde(default)]
  pub hostname: String,

  /// The Deployment / Stack this hostname serves. Optional: a Domain
  /// without a target is a free-form DNS / tunnel declaration.
  /// Also picks the tunnel bound to the Server it runs on.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub target: Option<ResourceTarget>,

  /// Service name inside the target Stack. Empty for Deployments.
  #[serde(default)]
  pub service: String,

  /// Published port of the target service. 0 = unknown.
  #[serde(default)]
  pub port: u16,

  /// DNS record settings. Leave `account` empty to not manage DNS.
  #[serde(default)]
  pub dns: DnsBinding,

  /// Tunnel / ingress settings. Leave `account` empty to only manage DNS
  /// (eg. point the hostname at an IP or another CNAME).
  #[serde(default)]
  pub ingress: IngressBinding,

  /// What to do with remote objects created by Komodo when
  /// this Domain is deleted. Default: `Cleanup`.
  #[serde(default)]
  pub on_delete: OnDelete,
}

#[cfg(feature = "utoipa")]
impl utoipa::PartialSchema for PartialDomainConfig {
  fn schema()
  -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    utoipa::schema!(#[inline] std::collections::HashMap<String, serde_json::Value>).into()
  }
}

#[cfg(feature = "utoipa")]
impl utoipa::ToSchema for PartialDomainConfig {}

/// DNS record settings of a Domain.
#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Default, PartialEq,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DnsBinding {
  /// Name of the Domain account which manages DNS. Empty = DNS not managed.
  #[serde(default)]
  pub account: String,
  /// Default: `CNAME`
  #[serde(default)]
  pub record_type: DnsRecordType,
  /// Record content. If empty, taken from the ingress tunnel
  /// (eg. `<tunnel-id>.cfargotunnel.com`).
  #[serde(default)]
  pub content: String,
  /// TTL seconds. 0 / unset = provider automatic.
  #[serde(default)]
  pub ttl: u32,
  /// Whether to proxy through the provider. Unset = provider decides
  /// (proxied when pointing at a tunnel).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub proxied: Option<bool>,
}

/// Tunnel / ingress settings of a Domain.
#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Default, PartialEq,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct IngressBinding {
  /// Name of the Domain account which owns the tunnel.
  /// Empty = ingress not managed.
  #[serde(default)]
  pub account: String,
  /// Name of a tunnel registered on the account.
  /// If empty, picked automatically from the Server the Domain `target` runs on.
  #[serde(default)]
  pub tunnel: String,
  /// Origin service, eg. `http://localhost:8080`.
  #[serde(default)]
  pub service: String,
}

#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum DnsRecordType {
  A,
  AAAA,
  #[default]
  CNAME,
}

impl DnsRecordType {
  pub fn as_str(&self) -> &'static str {
    match self {
      Self::A => "A",
      Self::AAAA => "AAAA",
      Self::CNAME => "CNAME",
    }
  }
}

#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum OnDelete {
  /// Remove remote objects created by Komodo. Delete is refused if cleanup fails.
  #[default]
  Cleanup,
  /// Leave remote objects untouched.
  Retain,
}

// INFO

/// System maintained state, not user configurable.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DomainInfo {
  /// Last observed status. `None` = never checked.
  #[serde(default)]
  pub status: Option<DomainStatus>,
  /// When status was last observed (ms).
  #[serde(default)]
  pub observed_at: I64,
  /// Remote objects created and claimed by Komodo.
  #[serde(default)]
  pub owned: Vec<OwnedObject>,
}

#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash,
)]
pub enum DomainLayer {
  Dns,
  Ingress,
}

/// A remote object Komodo created. Drift and cleanup only consider these.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OwnedObject {
  pub layer: DomainLayer,
  /// The Domain account name used to create it.
  pub account: String,
  /// Hostname the object serves.
  pub key: String,
  /// Provider object id (DNS record id; empty for tunnel rules).
  pub remote_id: String,
  /// Opaque provider container (zone id / tunnel id). Needed for cleanup.
  pub scope: String,
  /// Digest of the user controlled fields as last written by Komodo.
  pub applied_digest: String,
}

#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum DomainStatus {
  Ready,
  Drifted { reasons: Vec<DomainReason> },
  Error { reason: DomainReason },
}

impl DomainStatus {
  pub fn code(&self) -> &'static str {
    match self {
      Self::Ready => "ready",
      Self::Drifted { .. } => "drifted",
      Self::Error { .. } => "error",
    }
  }
}

/// A machine readable problem. `code` is stable.
///
/// Codes: `AccountNotFound` `AccountKindMismatch` `InvalidConfig`
/// `ZoneNotFound` `TunnelNotFound` `TunnelAmbiguous` `AuthFailed`
/// `RateLimited` `ProviderUnavailable` `ProviderRejected`
/// `ProviderResponse` `Timeout` `RecordMissing` `RecordChanged`
/// `RuleMissing` `RuleChanged` `ForeignRecordConflict` `PlanChanged`
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DomainReason {
  pub code: String,
  pub layer: Option<DomainLayer>,
  pub object_key: Option<String>,
  /// Human readable summary. Never contains tokens or provider response bodies.
  pub message: String,
  pub retryable: bool,
}

// PLAN

#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DomainPlan {
  /// Domain id
  pub domain: String,
  pub generated_at: I64,
  /// Digest of `changes`. Pass to ApplyDomain to ensure what is applied is what was reviewed.
  pub plan_id: String,
  pub changes: Vec<Change>,
  /// Problems which prevent computing / applying the plan.
  pub errors: Vec<DomainReason>,
}

#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Change {
  pub layer: DomainLayer,
  pub object: ObjectRef,
  pub action: ChangeAction,
  /// Normalized current remote content
  pub before: Option<JsonValue>,
  /// Normalized desired content
  pub after: Option<JsonValue>,
  pub reason: Option<ChangeReason>,
}

#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ObjectRef {
  /// `dns_record` | `tunnel_rule`
  pub kind: String,
  /// The hostname
  pub key: String,
  pub remote_id: Option<String>,
}

#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq,
)]
pub enum ChangeAction {
  Create,
  Update,
  Delete,
  Noop,
  /// Remote object exists but is not owned by Komodo.
  /// Apply refuses unless `adopt_conflicts` is set.
  Conflict,
}

#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq,
)]
pub enum ChangeReason {
  Missing,
  ContentDiffers,
  /// Object created by Komodo was edited remotely.
  ManualEdit,
  NotOwned,
  HostnameChanged,
  /// Owned object no longer declared by the config.
  OrphanOwned,
}

// SERVICE DOMAINS

/// How a hostname reaches the service.
#[typeshare]
#[derive(
  Serialize,
  Deserialize,
  Debug,
  Clone,
  Copy,
  PartialEq,
  Eq,
  Default,
  clap::ValueEnum,
)]
pub enum DomainMode {
  /// Tunnel if the target's Server has one, else DNS.
  #[default]
  Auto,
  Tunnel,
  Dns,
}

/// Result of AddServiceDomain.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ServiceDomainResult {
  /// Set once the Domain was created (and applied).
  pub domain: Option<Domain>,
  /// `Tunnel` or `Dns`, whichever was chosen.
  pub mode: Option<DomainMode>,
  /// Remote objects which exist but are not managed by Komodo.
  /// Non empty + `domain` unset = confirm, then retry with `adopt_conflicts`.
  pub conflicts: Vec<Change>,
  /// Plain language notes, eg. LAN only.
  pub warnings: Vec<String>,
  /// Why nothing was done / what failed, in plain language.
  pub error: Option<String>,
  /// Machine readable apply failure, if apply started and failed.
  pub reason: Option<DomainReason>,
}

/// Where `ExposeService` currently is. Only set while it runs.
#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq,
)]
pub enum ExposeStage {
  /// Opening the connection of the Server.
  Connecting,
  /// Setting up the public address.
  Configuring,
}

/// A published port of a service.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ServicePort {
  /// Port on the Server.
  pub host: u16,
  /// Port inside the container.
  pub container: u16,
  /// `http://<server host>:<host port>`. Unset when the Server has no
  /// usable address.
  pub lan_url: Option<String>,
}

/// A public address of a service.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServicePublicUrl {
  /// `https://<hostname>`
  pub url: String,
  /// Name of the backing Domain resource.
  pub domain: String,
  pub domain_id: String,
  pub port: u16,
  /// `ready` | `drifted` | `error` | `unchecked`
  pub status: String,
}

/// One Deployment, or one service of a Stack.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServiceItem {
  /// The Deployment, or the Stack of the service.
  pub target: ResourceTarget,
  /// Stack service name. Empty for a Deployment.
  pub service: String,
  /// Deployment name, or Stack service name.
  pub name: String,
  /// Stack name. Empty for a Deployment.
  pub stack: String,
  pub server_id: String,
  pub server_name: String,
  /// Container state, eg. `Running`, `Exited`.
  pub state: String,
  pub ports: Vec<ServicePort>,
  pub public: Vec<ServicePublicUrl>,
  /// Set while `ExposeService` runs for this service.
  pub expose: Option<ExposeStage>,
}

/// Result of ExposeService.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ExposeServiceResult {
  /// `https://...` once the service is public.
  pub url: Option<String>,
  /// Remote objects which exist but are not managed by Komodo.
  /// Non empty + `url` unset = confirm, then retry with `adopt_conflicts`.
  pub conflicts: Vec<Change>,
  /// Why nothing was done / what failed, with the next step.
  pub error: Option<String>,
}

// QUERY

#[typeshare]
pub type DomainQuery = ResourceQuery<DomainQuerySpecifics>;

#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Default, DefaultBuilder,
)]
pub struct DomainQuerySpecifics {}

impl AddFilters for DomainQuerySpecifics {
  fn add_filters(&self, _filters: &mut Document) {}
}
