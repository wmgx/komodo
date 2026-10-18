//! Compile-time driver registry. Business code uses these versioned interfaces;
//! provider identity, credential resolution and protocol dispatch stay here.
use super::*;
use komodo_client::entities::domain::{
  DnsProvider, DomainAdapterCapabilities, ProxyMode, TlsMode,
};

pub const REGISTERED_ADAPTERS: &[&str] = &[
  "CloudflareDnsV1",
  "ExternalDns",
  "TraefikFileV1",
  "CloudflareTunnelV1",
];

pub fn capabilities(
  adapter: &str,
) -> Result<DomainAdapterCapabilities> {
  if !REGISTERED_ADAPTERS.contains(&adapter) {
    return Err(AdapterError::new(
      "UnsupportedCapability",
      "Unknown adapter or schema version",
      false,
    ));
  }
  let dns = matches!(adapter, "CloudflareDnsV1" | "ExternalDns");
  Ok(DomainAdapterCapabilities {
    adapter: adapter.into(),
    schema_version: 1,
    target_types: if dns {
      vec![]
    } else if adapter == "TraefikFileV1" {
      vec!["Url".into(), "Deployment".into(), "StackService".into()]
    } else {
      vec!["Url".into()]
    },
    tls_modes: if dns {
      vec![]
    } else {
      vec![TlsMode::RequireHttps, TlsMode::HttpOnly]
    },
    dns_types: if dns {
      vec!["A".into(), "AAAA".into(), "CNAME".into()]
    } else {
      vec![]
    },
    proxy_modes: if adapter == "CloudflareDnsV1" {
      vec![ProxyMode::DnsOnly, ProxyMode::Proxied]
    } else {
      vec![]
    },
    migration_modes: vec!["PrepareThenCutover".into()],
    tls_bootstrap_requires_dns: adapter == "CloudflareTunnelV1",
  })
}

/// Enum dispatch is deliberately used instead of an async trait object. Adding
/// an implementation extends this registry, not the Domain or CLI model.
pub enum DnsDriver {
  CloudflareDnsV1 {
    client: CloudflareClient,
    zone: String,
    account: String,
    suffixes: Vec<String>,
  },
}

#[derive(Debug, Clone)]
pub struct DnsPlan {
  pub desired: Value,
  pub fingerprint: String,
  pub changes: bool,
}

impl DnsDriver {
  pub fn from_provider(provider: &DnsProvider) -> Result<Self> {
    let config = &provider.config;
    if config.adapter != "CloudflareDnsV1"
      || config.schema_version != 1
    {
      return Err(AdapterError::new(
        "UnsupportedCapability",
        "DNS adapter or schema version is not installed",
        false,
      ));
    }
    segment(&config.zone_id)?;
    segment(&config.account_id)?;
    if config.allowed_hostname_suffixes.is_empty() {
      return Err(AdapterError::new(
        "PermissionDenied",
        "DNS hostname scope must not be empty",
        false,
      ));
    }
    Ok(Self::CloudflareDnsV1 {
      client: cloudflare_from_secret(&config.secret)?,
      zone: config.zone_id.clone(),
      account: config.account_id.clone(),
      suffixes: config.allowed_hostname_suffixes.clone(),
    })
  }
  pub fn capabilities(&self) -> DomainAdapterCapabilities {
    match self {
      Self::CloudflareDnsV1 { .. } => {
        capabilities("CloudflareDnsV1").expect("registered adapter")
      }
    }
  }
  fn check_scope(&self, hostname: &str) -> Result<()> {
    let Self::CloudflareDnsV1 { suffixes, .. } = self;
    let hostname =
      hostname.trim_end_matches('.').to_ascii_lowercase();
    if !suffixes.iter().any(|suffix| {
      let suffix =
        match url::Host::parse(suffix.trim_end_matches('.')) {
          Ok(url::Host::Domain(host)) => host.to_ascii_lowercase(),
          _ => return false,
        };
      !suffix.is_empty()
        && (hostname == suffix
          || hostname.ends_with(&format!(".{suffix}")))
    }) {
      return Err(AdapterError::new(
        "PermissionDenied",
        "Hostname is outside the configured DNS scope",
        false,
      ));
    }
    Ok(())
  }
  pub async fn validate(&self) -> Result<()> {
    match self {
      Self::CloudflareDnsV1 {
        client,
        zone,
        account,
        ..
      } => {
        client.validate_zone(zone, account).await?;
        Ok(())
      }
    }
  }
  async fn validate_hostname(&self, hostname: &str) -> Result<()> {
    self.check_scope(hostname)?;
    match self {
      Self::CloudflareDnsV1 {
        client,
        zone,
        account,
        ..
      } => {
        let details = client.validate_zone(zone, account).await?;
        let zone_name =
          details["name"].as_str().ok_or_else(malformed)?;
        let host =
          hostname.trim_end_matches('.').to_ascii_lowercase();
        let zone_name =
          zone_name.trim_end_matches('.').to_ascii_lowercase();
        if host != zone_name
          && !host.ends_with(&format!(".{zone_name}"))
        {
          return Err(AdapterError::new(
            "PermissionDenied",
            "Hostname does not belong to the authoritative provider zone",
            false,
          ));
        }
        Ok(())
      }
    }
  }
  pub async fn observe(&self, hostname: &str) -> Result<Vec<Value>> {
    self.validate_hostname(hostname).await?;
    match self {
      Self::CloudflareDnsV1 { client, zone, .. } => {
        client.list_dns(zone, hostname).await
      }
    }
  }
  /// Read-only prediction over already-observed state. No request or mutation.
  pub fn plan(
    &self,
    intent: &DnsIntent,
    observed: &[Value],
    owned: Option<&OwnedObject>,
    owner: &str,
  ) -> Result<DnsPlan> {
    self.check_scope(&intent.hostname)?;
    if !matches!(intent.record_type.as_str(), "A" | "AAAA" | "CNAME")
    {
      return Err(AdapterError::new(
        "UnsupportedCapability",
        "Unsupported DNS type",
        false,
      ));
    }
    let old = owned.and_then(|h| {
      observed.iter().find(|r| r["id"].as_str() == Some(&h.id))
    });
    let mut desired = old.cloned().unwrap_or_else(|| json!({}));
    desired["name"] = json!(intent.hostname);
    desired["type"] = json!(intent.record_type);
    desired["content"] =
      json!(dns_canonical_content(&intent.content));
    desired["ttl"] =
      json!(if intent.proxied { 1 } else { intent.ttl });
    desired["proxied"] = json!(intent.proxied);
    desired["comment"] = json!(owner);
    let fingerprint = dns_fingerprint(&desired);
    let changes =
      old.is_none_or(|r| dns_fingerprint(r) != fingerprint);
    Ok(DnsPlan {
      desired,
      fingerprint,
      changes,
    })
  }
  pub async fn apply(
    &self,
    intent: &DnsIntent,
    owned: Option<&OwnedObject>,
    owner: &str,
  ) -> Result<OwnedObject> {
    bounded_write(async {
      self.validate_hostname(&intent.hostname).await?;
      match self {
        Self::CloudflareDnsV1 { client, zone, .. } => {
          client.apply_dns(zone, intent, owned, owner).await
        }
      }
    })
    .await
  }
  pub async fn remove_owned(
    &self,
    owned: &OwnedObject,
  ) -> Result<()> {
    match self {
      Self::CloudflareDnsV1 { client, zone, .. } => {
        client.remove_dns(zone, owned).await
      }
    }
  }
  pub async fn adopt(
    &self,
    object_id: &str,
    expected_fingerprint: &str,
    owner: &str,
  ) -> Result<OwnedObject> {
    match self {
      Self::CloudflareDnsV1 { client, zone, .. } => {
        client
          .adopt_dns(zone, object_id, expected_fingerprint, owner)
          .await
      }
    }
  }
  pub async fn strict_origin_tls(&self) -> Result<bool> {
    match self {
      Self::CloudflareDnsV1 { client, zone, .. } => {
        client.strict_origin_tls(zone).await
      }
    }
  }
}

use komodo_client::entities::domain::{
  DomainTls, Ingress, IngressAdapterConfig,
};

pub enum IngressDriver {
  TraefikFileV1 {
    client: crate::periphery::PeripheryClient,
    resource: Ingress,
  },
  CloudflareTunnelV1 {
    client: CloudflareClient,
    resource: Ingress,
  },
}
#[derive(Debug, Clone)]
pub struct IngressPlan {
  pub desired: Value,
  /// Tunnel supplies a post-write handle. Traefik publication is attested by Periphery.
  pub handle: Option<OwnedObject>,
  pub dns_target: String,
  pub requires_downtime: bool,
}
impl IngressDriver {
  pub async fn from_ingress(resource: &Ingress) -> Result<Self> {
    if resource.config.schema_version != 1 {
      return Err(AdapterError::new(
        "UnsupportedCapability",
        "Ingress schema version is not installed",
        false,
      ));
    }
    match &resource.config.adapter {
      IngressAdapterConfig::TraefikFileV1 {
        server,
        dedicated,
        ..
      } => {
        if !dedicated {
          return Err(AdapterError::new(
            "UnsupportedCapability",
            "A dedicated Traefik entrance is required",
            false,
          ));
        }
        let server = crate::resource::get::<
          komodo_client::entities::server::Server,
        >(server)
        .await
        .map_err(|_| {
          AdapterError::new(
            "NotFound",
            "Ingress server is unavailable",
            false,
          )
        })?;
        let client = crate::helpers::periphery_client(&server)
          .await
          .map_err(periphery_error)?;
        Ok(Self::TraefikFileV1 {
          client,
          resource: resource.clone(),
        })
      }
      IngressAdapterConfig::CloudflareTunnelV1 {
        secret,
        dedicated,
        ..
      } => {
        if !dedicated {
          return Err(AdapterError::new(
            "UnsupportedCapability",
            "A dedicated single-writer Tunnel is required",
            false,
          ));
        }
        Ok(Self::CloudflareTunnelV1 {
          client: cloudflare_from_secret(secret)?,
          resource: resource.clone(),
        })
      }
    }
  }
  pub fn capabilities(&self) -> DomainAdapterCapabilities {
    capabilities(match self {
      Self::TraefikFileV1 { .. } => "TraefikFileV1",
      Self::CloudflareTunnelV1 { .. } => "CloudflareTunnelV1",
    })
    .expect("registered adapter")
  }
  pub fn dns_target(&self) -> String {
    match self {
      Self::TraefikFileV1 { resource, .. } => {
        match &resource.config.adapter {
          IngressAdapterConfig::TraefikFileV1 {
            dns_target, ..
          } => dns_target.clone(),
          _ => unreachable!(),
        }
      }
      Self::CloudflareTunnelV1 { resource, .. } => {
        match &resource.config.adapter {
          IngressAdapterConfig::CloudflareTunnelV1 {
            tunnel_id,
            ..
          } => format!("{tunnel_id}.cfargotunnel.com"),
          _ => unreachable!(),
        }
      }
    }
  }
  pub async fn validate(&self) -> Result<()> {
    match self {
      Self::TraefikFileV1 { client, resource } => {
        TraefikFileDriver {
          client,
          ingress_id: &resource.id,
        }
        .validate()
        .await
      }
      Self::CloudflareTunnelV1 { client, resource } => {
        let IngressAdapterConfig::CloudflareTunnelV1 {
          account_id,
          tunnel_id,
          ..
        } = &resource.config.adapter
        else {
          unreachable!()
        };
        client.observe_tunnel(account_id, tunnel_id).await?;
        if !client.tunnel_connected(account_id, tunnel_id).await? {
          return Err(AdapterError::new(
            "IngressUnavailable",
            "Tunnel has no healthy connector",
            true,
          ));
        }
        Ok(())
      }
    }
  }
  pub async fn observe(&self, binding_id: &str) -> Result<Value> {
    match self {
      Self::TraefikFileV1 { client, resource } => {
        serde_json::to_value(
          TraefikFileDriver {
            client,
            ingress_id: &resource.id,
          }
          .observe(binding_id)
          .await?,
        )
        .map_err(|_| malformed())
      }
      Self::CloudflareTunnelV1 { client, resource } => {
        let IngressAdapterConfig::CloudflareTunnelV1 {
          account_id,
          tunnel_id,
          ..
        } = &resource.config.adapter
        else {
          unreachable!()
        };
        client.observe_tunnel(account_id, tunnel_id).await
      }
    }
  }
  fn traefik_route(
    resource: &Ingress,
    route: &RouteSpec,
    tls: &DomainTls,
  ) -> Result<periphery_client::api::domain::DomainRoute> {
    let profile = resource
      .config
      .tls_profiles
      .iter()
      .find(|p| p.name == tls.profile && p.mode == tls.mode)
      .ok_or_else(|| {
        AdapterError::new(
          "UnsupportedCapability",
          "TLS profile is unavailable",
          false,
        )
      })?;
    let IngressAdapterConfig::TraefikFileV1 { entrypoint, .. } =
      &resource.config.adapter
    else {
      unreachable!()
    };
    Ok(periphery_client::api::domain::DomainRoute {
      hostname: route.hostname.clone(),
      backend_url: route.backend_url.clone(),
      entrypoints: vec![entrypoint.clone()],
      require_https: tls.mode == TlsMode::RequireHttps,
      certificate_resolver: (!profile
        .certificate_resolver
        .is_empty())
      .then(|| profile.certificate_resolver.clone()),
    })
  }
  pub fn plan(
    &self,
    route: &RouteSpec,
    tls: &DomainTls,
    observed: &Value,
    owned: Option<&OwnedObject>,
  ) -> Result<IngressPlan> {
    let (desired, handle, requires_downtime) = match self {
      Self::TraefikFileV1 { resource, .. } => {
        let spec = Self::traefik_route(resource, route, tls)?;
        (
          serde_json::to_value(spec).map_err(|_| malformed())?,
          None,
          resource
            .config
            .tls_profiles
            .iter()
            .any(|p| p.name == tls.profile && p.requires_dns),
        )
      }
      Self::CloudflareTunnelV1 { .. } => {
        let desired = merge_tunnel(observed, route, owned, false)?;
        let rule = desired["ingress"]
          .as_array()
          .and_then(|rules| {
            rules.iter().find(|r| r["hostname"] == route.hostname)
          })
          .ok_or_else(malformed)?;
        let handle = OwnedObject {
          id: route.hostname.clone(),
          owner: route.binding_id.clone(),
          fingerprint: fingerprint(rule),
        };
        (desired, Some(handle), true)
      }
    };
    Ok(IngressPlan {
      desired,
      handle,
      dns_target: self.dns_target(),
      requires_downtime,
    })
  }
  pub async fn apply(
    &self,
    route: &RouteSpec,
    tls: &DomainTls,
    owned: Option<&OwnedObject>,
    observed: &Value,
    fence: u64,
  ) -> Result<OwnedObject> {
    match self {
      Self::TraefikFileV1 { client, resource } => {
        let spec = Self::traefik_route(resource, route, tls)?;
        let result = TraefikFileDriver {
          client,
          ingress_id: &resource.id,
        }
        .apply(
          &route.binding_id,
          fence,
          owned.map(|h| h.fingerprint.clone()),
          spec,
        )
        .await?;
        Ok(OwnedObject {
          id: route.binding_id.clone(),
          owner: route.binding_id.clone(),
          fingerprint: result.fingerprint.ok_or_else(malformed)?,
        })
      }
      Self::CloudflareTunnelV1 { client, resource } => {
        let IngressAdapterConfig::CloudflareTunnelV1 {
          account_id,
          tunnel_id,
          ..
        } = &resource.config.adapter
        else {
          unreachable!()
        };
        client
          .apply_tunnel_route(
            account_id,
            tunnel_id,
            &fingerprint(observed),
            route,
            owned,
          )
          .await
      }
    }
  }
  /// Predict the exact post-removal shared configuration before a bounded write.
  pub fn plan_remove(
    &self,
    binding_id: &str,
    owned: &OwnedObject,
    observed: &Value,
  ) -> Result<Value> {
    match self {
      Self::TraefikFileV1 { .. } => Ok(Value::Null),
      Self::CloudflareTunnelV1 { .. } => merge_tunnel(
        observed,
        &RouteSpec {
          binding_id: binding_id.into(),
          hostname: owned.id.clone(),
          backend_url: String::new(),
        },
        Some(owned),
        true,
      ),
    }
  }
  pub async fn remove_owned(
    &self,
    binding_id: &str,
    owned: &OwnedObject,
    observed: &Value,
    fence: u64,
  ) -> Result<()> {
    match self {
      Self::TraefikFileV1 { client, resource } => {
        TraefikFileDriver {
          client,
          ingress_id: &resource.id,
        }
        .remove_owned(
          binding_id,
          fence,
          Some(owned.fingerprint.clone()),
        )
        .await?;
        Ok(())
      }
      Self::CloudflareTunnelV1 { client, resource } => {
        let IngressAdapterConfig::CloudflareTunnelV1 {
          account_id,
          tunnel_id,
          ..
        } = &resource.config.adapter
        else {
          unreachable!()
        };
        client
          .remove_tunnel_route(
            account_id,
            tunnel_id,
            &fingerprint(observed),
            owned,
          )
          .await
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  fn dns() -> DnsDriver {
    DnsDriver::CloudflareDnsV1 {
      client: CloudflareClient::new("test-only-placeholder").unwrap(),
      zone: "zone".into(),
      account: "account".into(),
      suffixes: vec!["example.com".into()],
    }
  }
  #[test]
  fn registry_rejects_unknown_version_and_scopes_plan() {
    assert!(capabilities("FutureDnsV99").is_err());
    let intent = DnsIntent {
      hostname: "evil-example.com".into(),
      record_type: "A".into(),
      content: "192.0.2.1".into(),
      ttl: 60,
      proxied: false,
    };
    assert_eq!(
      dns().plan(&intent, &[], None, "owner").unwrap_err().code,
      "PermissionDenied"
    );
  }
  #[test]
  fn dns_plan_predicts_same_canonical_state_without_writes() {
    let record = json!({"id":"record","name":"app.example.com","type":"AAAA","content":"2001:db8::1","ttl":60,"proxied":false,"comment":"owner","settings":{},"tags":[]});
    let handle = OwnedObject {
      id: "record".into(),
      owner: "owner".into(),
      fingerprint: dns_fingerprint(&record),
    };
    let intent = DnsIntent {
      hostname: "app.example.com".into(),
      record_type: "AAAA".into(),
      content: "2001:0db8:0000::0001".into(),
      ttl: 60,
      proxied: false,
    };
    let plan = dns()
      .plan(&intent, &[record], Some(&handle), "owner")
      .unwrap();
    assert!(!plan.changes);
    assert_eq!(plan.fingerprint, handle.fingerprint);
  }
}
