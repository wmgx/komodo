use std::fmt;

use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use super::deployment::DeploymentState;
use super::domain::DomainLayer;
use super::{JsonValue, MongoId, config::empty_or_redacted};

/// Stored account (Mongo document). Holds the token in plain text,
/// like git provider accounts. Never returned from the API:
/// see [DomainAccount] for the redacted view.
#[derive(Serialize, Deserialize, Clone, Default)]
#[cfg_attr(
  feature = "mongo",
  derive(mongo_indexed::derive::MongoIndexed)
)]
#[cfg_attr(feature = "mongo", unique_doc_index({ "name": 1 }))]
pub struct DomainAccountDoc {
  #[serde(
    default,
    rename = "_id",
    skip_serializing_if = "String::is_empty",
    with = "bson::serde_helpers::hex_string_as_object_id"
  )]
  pub id: MongoId,
  /// Globally unique. Domains reference accounts by name.
  pub name: String,
  /// Provider kind, eg. `cloudflare`.
  pub kind: String,
  /// API token in plain text.
  #[serde(default)]
  pub token: String,
  /// Non-secret provider parameters. Never put credentials here.
  #[serde(default)]
  pub params: JsonValue,
  #[serde(default)]
  pub description: String,
}

impl fmt::Debug for DomainAccountDoc {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("DomainAccountDoc")
      .field("id", &self.id)
      .field("name", &self.name)
      .field("kind", &self.kind)
      .field("token", &"***")
      .finish()
  }
}

/// Account declared in the Core config (`[[domain_account]]`).
/// Database accounts take priority over config ones with the same name.
/// This is NOT part of core `secrets`, so it is never interpolated.
#[typeshare]
#[derive(Serialize, Deserialize, Clone, Default)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DomainAccountConfig {
  pub name: String,
  pub kind: String,
  #[serde(default, skip_serializing)]
  pub token: String,
  #[serde(default)]
  #[cfg_attr(feature = "utoipa", schema(value_type = Object))]
  pub params: JsonValue,
  #[serde(default)]
  pub description: String,
}

impl DomainAccountConfig {
  pub fn sanitized(&self) -> DomainAccountConfig {
    DomainAccountConfig {
      token: empty_or_redacted(&self.token),
      ..self.clone()
    }
  }
}

impl fmt::Debug for DomainAccountConfig {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("DomainAccountConfig")
      .field("name", &self.name)
      .field("kind", &self.kind)
      .field("token", &"***")
      .finish()
  }
}

#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq,
)]
pub enum DomainAccountSource {
  Database,
  /// Declared in Core config, read only.
  Config,
}

/// Redacted account view returned by the API. Never contains the token.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DomainAccount {
  /// Empty for config accounts.
  pub id: String,
  pub name: String,
  pub kind: String,
  pub params: JsonValue,
  pub description: String,
  pub token_set: bool,
  pub source: DomainAccountSource,
  /// What this account kind can do.
  pub capabilities: Vec<DomainLayer>,
}

/// An available provider kind.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DomainProviderKind {
  pub kind: String,
  pub capabilities: Vec<DomainLayer>,
  /// Example `params` for accounts of this kind.
  pub params_example: JsonValue,
}

/// What a provider token can see. Vendor neutral.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DomainDiscovery {
  /// DNS zone names.
  pub zones: Vec<String>,
  /// Tunnels / entrypoints that can receive ingress rules.
  pub ingresses: Vec<DiscoveredIngress>,
  /// Non fatal problems, eg. a missing permission.
  pub warnings: Vec<String>,
}

#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiscoveredIngress {
  /// Provider id (eg. tunnel id).
  pub id: String,
  pub name: String,
  /// Provider container (eg. Cloudflare account id).
  pub account_id: Option<String>,
  pub status: Option<String>,
}

/// Overall state of the tunnel on a Server, in plain terms.
#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default,
)]
pub enum ServerTunnelState {
  /// No tunnel is bound to the Server.
  #[default]
  NotEnabled,
  /// The connector is being deployed or is still connecting.
  Starting,
  Healthy,
  /// Connected, but not with all connections.
  Degraded,
  Down,
  /// Registered, but nothing is connected (connector missing / stopped).
  Inactive,
}

/// The tunnel of one Server and the connector (cloudflared) running it.
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ServerTunnel {
  pub state: ServerTunnelState,
  /// Account which owns the tunnel, or the one which would be used.
  pub account: Option<String>,
  /// Accounts which can open a tunnel by themselves. More than one means
  /// the caller has to choose.
  pub accounts: Vec<String>,
  pub tunnel_name: Option<String>,
  /// Created by Komodo (so Komodo may also delete it).
  pub managed: bool,
  /// Provider side connection status, eg. `healthy`.
  pub remote_status: Option<String>,
  /// Name / id of the connector Deployment.
  pub deployment: Option<String>,
  pub deployment_id: Option<String>,
  pub deployment_state: Option<DeploymentState>,
  /// Connector image which would be used (account default).
  pub image: String,
  /// Domains which currently use the tunnel.
  pub domains: Vec<String>,
  /// Plain language note about something half done / unreadable.
  pub warning: Option<String>,
}

#[cfg(test)]
mod tests {
  use crate::entities::config::core::CoreConfig;

  use super::DomainAccountConfig;

  #[test]
  fn core_config_sanitized_hides_domain_account_token() {
    let account = DomainAccountConfig {
      name: "cf".into(),
      kind: "cloudflare".into(),
      token: "super-secret-token".into(),
      ..Default::default()
    };
    let config = CoreConfig {
      domain_accounts: vec![account.clone()],
      ..Default::default()
    };
    let sanitized = config.sanitized();
    assert_ne!(sanitized.domain_accounts[0].token, account.token);
    assert!(!format!("{sanitized:?}").contains("super-secret-token"));
    assert!(!format!("{account:?}").contains("super-secret-token"));
  }
}
