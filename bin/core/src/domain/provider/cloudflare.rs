//! Cloudflare provider: DNS records + Cloudflare Tunnel (remote-managed) ingress rules.
//!
//! One account (one API token) provides both capabilities.
//!
//! Account `params`:
//! ```json
//! {
//!   "zones": ["example.com"],
//!   "tunnels": [
//!     { "name": "home", "account_id": "...", "tunnel_id": "...", "server": "my-server" }
//!   ]
//! }
//! ```
//! - `zones` is an optional allow-list (zone names or ids). Empty = any zone the token sees.
//! - `server` is an optional Komodo Server name or id the tunnel runs on.

use std::sync::Arc;

use komodo_client::entities::{
  JsonValue,
  domain::{DnsRecordType, DomainLayer, OwnedObject},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::error::{ProviderError, code};

use super::{
  BoxFut, Credentials, DnsProvider, DnsRecordSpec, DnsTarget,
  IngressProvider, IngressTarget, Owner, ProviderFactory,
  ProviderSet, RemoteRecord, RemoteRule, ServerRef,
  cloudflare_client::{CloudflareClient, Zone, segment},
  dns_canonical, tunnel_merge,
};

pub const FACTORY: ProviderFactory = ProviderFactory {
  kind: "cloudflare",
  capabilities: &[DomainLayer::Dns, DomainLayer::Ingress],
  params_example,
  validate_params,
  build,
};

fn params_example() -> JsonValue {
  json!({
    "zones": ["example.com"],
    "tunnels": [{
      "name": "home",
      "account_id": "<cloudflare account id>",
      "tunnel_id": "<tunnel uuid>",
      "server": "<optional komodo server name or id>"
    }]
  })
}

#[derive(Deserialize, Default, Clone)]
struct CloudflareParams {
  #[serde(default)]
  zones: Vec<String>,
  #[serde(default)]
  tunnels: Vec<TunnelEntry>,
}

#[derive(Deserialize, Clone)]
struct TunnelEntry {
  name: String,
  account_id: String,
  tunnel_id: String,
  #[serde(default)]
  server: Option<String>,
}

fn parse_params(
  params: &JsonValue,
) -> Result<CloudflareParams, ProviderError> {
  if params.is_null() {
    return Ok(CloudflareParams::default());
  }
  CloudflareParams::deserialize(params).map_err(|e| {
    ProviderError::invalid(format!("Invalid cloudflare params: {e}"))
  })
}

fn validate_params(params: &JsonValue) -> Result<(), ProviderError> {
  let params = parse_params(params)?;
  if params.zones.iter().any(|z| z.trim().is_empty()) {
    return Err(ProviderError::invalid("Empty entry in 'zones'"));
  }
  let mut names = std::collections::HashSet::new();
  for tunnel in &params.tunnels {
    if tunnel.name.trim().is_empty()
      || !names.insert(tunnel.name.as_str())
    {
      return Err(ProviderError::invalid(
        "Tunnel names must be non-empty and unique",
      ));
    }
    segment(&tunnel.account_id)?;
    segment(&tunnel.tunnel_id)?;
  }
  Ok(())
}

fn build(creds: Credentials) -> Result<ProviderSet, ProviderError> {
  let params = parse_params(&creds.params)?;
  let client = Arc::new(CloudflareClient::new(&creds.token)?);
  Ok(ProviderSet {
    dns: Some(Box::new(CloudflareDns {
      client: client.clone(),
      zones: params.zones,
    })),
    ingress: Some(Box::new(CloudflareIngress {
      client,
      tunnels: params.tunnels,
    })),
  })
}

// ZONE

/// Longest-suffix zone match among the zones the account may use.
pub fn match_zone<'a>(
  zones: &'a [Zone],
  allow: &[String],
  hostname: &str,
) -> Result<&'a Zone, ProviderError> {
  let hostname = hostname.trim_end_matches('.').to_ascii_lowercase();
  zones
    .iter()
    .filter(|zone| {
      allow.is_empty()
        || allow.iter().any(|a| {
          a.eq_ignore_ascii_case(&zone.name) || a == &zone.id
        })
    })
    .filter(|zone| {
      hostname == zone.name
        || hostname.ends_with(&format!(".{}", zone.name))
    })
    .max_by_key(|zone| zone.name.len())
    .ok_or_else(|| {
      ProviderError::new(
        code::ZONE_NOT_FOUND,
        format!(
          "No accessible Cloudflare zone matches hostname '{hostname}'"
        ),
      )
    })
}

// DNS

struct CloudflareDns {
  client: Arc<CloudflareClient>,
  zones: Vec<String>,
}

impl CloudflareDns {
  async fn zone_for(
    &self,
    hostname: &str,
  ) -> Result<Zone, ProviderError> {
    let zones = self.client.list_zones().await?;
    match_zone(&zones, &self.zones, hostname).cloned()
  }
}

fn record_from(
  zone: &str,
  value: &Value,
) -> Result<RemoteRecord, ProviderError> {
  let bad = || {
    ProviderError::new(
      code::PROVIDER_RESPONSE,
      "Provider returned an invalid DNS record",
    )
  };
  let record_type = value["type"].as_str().ok_or_else(bad)?;
  let ttl = value["ttl"].as_u64().unwrap_or(1) as u32;
  Ok(RemoteRecord {
    remote_id: value["id"].as_str().ok_or_else(bad)?.to_string(),
    scope: zone.to_string(),
    canonical: dns_canonical(
      record_type,
      value["content"].as_str().ok_or_else(bad)?,
      ttl,
      value["proxied"].as_bool().unwrap_or(false),
    ),
    marker: value["comment"].as_str().map(str::to_string),
  })
}

impl DnsProvider for CloudflareDns {
  fn validate(
    &self,
    hostname: &str,
    spec: &DnsRecordSpec,
  ) -> Result<(), ProviderError> {
    if hostname.is_empty() || hostname.contains(['*', '/', ':']) {
      return Err(ProviderError::invalid(
        "An exact hostname is required",
      ));
    }
    let content_ok = match spec.record_type {
      DnsRecordType::A => {
        spec.content.parse::<std::net::Ipv4Addr>().is_ok()
      }
      DnsRecordType::AAAA => {
        spec.content.parse::<std::net::Ipv6Addr>().is_ok()
      }
      DnsRecordType::CNAME => {
        !spec.content.is_empty()
          && spec.content.bytes().all(|c| {
            c.is_ascii_alphanumeric()
              || matches!(c, b'-' | b'.' | b'_')
          })
      }
    };
    if !content_ok {
      return Err(ProviderError::invalid(format!(
        "Invalid {} record content",
        spec.record_type.as_str()
      )));
    }
    if spec.ttl != 1 && !(60..=86400).contains(&spec.ttl) {
      return Err(ProviderError::invalid(
        "TTL must be 0 (auto) or between 60 and 86400",
      ));
    }
    Ok(())
  }

  fn list<'a>(
    &'a self,
    hostname: &'a str,
  ) -> BoxFut<'a, Vec<RemoteRecord>> {
    Box::pin(async move {
      let zone = self.zone_for(hostname).await?;
      self
        .client
        .list_dns(&zone.id, hostname)
        .await?
        .iter()
        .filter(|r| {
          matches!(r["type"].as_str(), Some("A" | "AAAA" | "CNAME"))
        })
        .map(|r| record_from(&zone.id, r))
        .collect()
    })
  }

  fn upsert<'a>(
    &'a self,
    hostname: &'a str,
    spec: &'a DnsRecordSpec,
    owner: &'a Owner,
    existing: Option<&'a RemoteRecord>,
  ) -> BoxFut<'a, RemoteRecord> {
    Box::pin(async move {
      let body = json!({
        "type": spec.record_type.as_str(),
        "name": hostname,
        "content": spec.content,
        "ttl": if spec.proxied { 1 } else { spec.ttl },
        "proxied": spec.proxied,
        "comment": owner.marker(),
      });
      let (zone, result) = match existing {
        Some(existing) => (
          existing.scope.clone(),
          self
            .client
            .patch_dns(&existing.scope, &existing.remote_id, &body)
            .await?,
        ),
        None => {
          let zone = self.zone_for(hostname).await?;
          let result =
            self.client.create_dns(&zone.id, &body).await?;
          (zone.id, result)
        }
      };
      record_from(&zone, &result)
    })
  }

  fn delete<'a>(
    &'a self,
    owned: &'a OwnedObject,
    owner: &'a Owner,
  ) -> BoxFut<'a, ()> {
    Box::pin(async move {
      let Some(current) =
        self.client.get_dns(&owned.scope, &owned.remote_id).await?
      else {
        return Ok(());
      };
      let record = record_from(&owned.scope, &current)?;
      if !self.is_owned(&record, owner) {
        // Someone replaced / re-claimed it. Not ours to delete anymore.
        warn!(
          "Not deleting dns record {} for {}: ownership marker is gone",
          owned.remote_id, owned.key
        );
        return Ok(());
      }
      self.client.delete_dns(&owned.scope, &owned.remote_id).await
    })
  }
}

// INGRESS

struct CloudflareIngress {
  client: Arc<CloudflareClient>,
  tunnels: Vec<TunnelEntry>,
}

fn split_scope(scope: &str) -> Result<(&str, &str), ProviderError> {
  scope.split_once('/').ok_or_else(|| {
    ProviderError::invalid("Invalid tunnel scope on owned object")
  })
}

fn target_of(tunnel: &TunnelEntry) -> IngressTarget {
  IngressTarget {
    id: format!("{}/{}", tunnel.account_id, tunnel.tunnel_id),
    dns_target: Some(DnsTarget {
      record_type: DnsRecordType::CNAME,
      content: format!("{}.cfargotunnel.com", tunnel.tunnel_id),
      proxied: true,
    }),
  }
}

fn rule_of(
  config: &Value,
  hostname: &str,
) -> Result<Option<RemoteRule>, ProviderError> {
  let Some(rules) = config["ingress"].as_array() else {
    return Err(ProviderError::new(
      code::PROVIDER_RESPONSE,
      "Tunnel configuration has no ingress rules",
    ));
  };
  let mut matching = rules.iter().filter(|rule| {
    rule["hostname"]
      .as_str()
      .is_some_and(|h| h.eq_ignore_ascii_case(hostname))
  });
  let first = matching.next();
  if matching.next().is_some() {
    return Err(ProviderError::new(
      code::FOREIGN_CONFLICT,
      "Multiple rules already exist for this hostname",
    ));
  }
  Ok(first.map(|rule| RemoteRule {
    service: rule["service"].as_str().unwrap_or_default().to_string(),
    rule: rule.clone(),
  }))
}

impl IngressProvider for CloudflareIngress {
  fn validate(&self, service: &str) -> Result<(), ProviderError> {
    let ok = !service.is_empty()
      && !service
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
      && (service.contains("://")
        || service.starts_with("unix:")
        || service.starts_with("http_status:"));
    if ok {
      Ok(())
    } else {
      Err(ProviderError::invalid(
        "Ingress service must look like 'http://host:port'",
      ))
    }
  }

  fn select(
    &self,
    name: Option<&str>,
    server: Option<&ServerRef>,
  ) -> Result<IngressTarget, ProviderError> {
    if self.tunnels.is_empty() {
      return Err(ProviderError::new(
        code::TUNNEL_NOT_FOUND,
        "No tunnels are registered on this account",
      ));
    }
    if let Some(name) = name.filter(|n| !n.is_empty()) {
      return self
        .tunnels
        .iter()
        .find(|t| t.name == name)
        .map(target_of)
        .ok_or_else(|| {
          ProviderError::new(
            code::TUNNEL_NOT_FOUND,
            format!("No tunnel named '{name}' on this account"),
          )
        });
    }
    let candidates: Vec<&TunnelEntry> = match server {
      Some(server) => self
        .tunnels
        .iter()
        .filter(|t| {
          t.server
            .as_deref()
            .is_some_and(|s| s == server.id || s == server.name)
        })
        .collect(),
      // Nothing to disambiguate with: only an unambiguous account works.
      None => self.tunnels.iter().collect(),
    };
    match candidates.as_slice() {
      [one] => Ok(target_of(one)),
      [] => Err(ProviderError::new(
        code::TUNNEL_NOT_FOUND,
        "No tunnel is bound to the target's Server, set 'tunnel' explicitly",
      )),
      _ => Err(ProviderError::new(
        code::TUNNEL_AMBIGUOUS,
        "Multiple tunnels match, set 'tunnel' explicitly",
      )),
    }
  }

  fn get<'a>(
    &'a self,
    target: &'a IngressTarget,
    hostname: &'a str,
  ) -> BoxFut<'a, Option<RemoteRule>> {
    Box::pin(async move {
      let (account, tunnel) = split_scope(&target.id)?;
      let config = self.client.tunnel_config(account, tunnel).await?;
      rule_of(&config, hostname)
    })
  }

  fn put<'a>(
    &'a self,
    target: &'a IngressTarget,
    hostname: &'a str,
    service: &'a str,
  ) -> BoxFut<'a, RemoteRule> {
    Box::pin(async move {
      let (account, tunnel) = split_scope(&target.id)?;
      let config = self.client.tunnel_config(account, tunnel).await?;
      let next =
        tunnel_merge::merge(&config, hostname, service, false)?;
      if next != config {
        self
          .client
          .put_tunnel_config(account, tunnel, &next)
          .await?;
      }
      rule_of(&next, hostname)?.ok_or_else(|| {
        ProviderError::new(
          code::PROVIDER_RESPONSE,
          "Rule missing after merge",
        )
      })
    })
  }

  fn remove<'a>(&'a self, owned: &'a OwnedObject) -> BoxFut<'a, ()> {
    Box::pin(async move {
      let (account, tunnel) = split_scope(&owned.scope)?;
      let config = self.client.tunnel_config(account, tunnel).await?;
      let next = tunnel_merge::merge(&config, &owned.key, "", true)?;
      if next != config {
        self
          .client
          .put_tunnel_config(account, tunnel, &next)
          .await?;
      }
      Ok(())
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn zone(id: &str, name: &str) -> Zone {
    Zone {
      id: id.into(),
      name: name.into(),
    }
  }

  #[test]
  fn zone_longest_suffix_wins_and_unknown_fails() {
    let zones = vec![
      zone("1", "example.com"),
      zone("2", "dev.example.com"),
      zone("3", "other.org"),
    ];
    let m = match_zone(&zones, &[], "app.dev.example.com").unwrap();
    assert_eq!(m.id, "2");
    let m = match_zone(&zones, &[], "example.com").unwrap();
    assert_eq!(m.id, "1");
    // allow-list excludes the more specific zone
    let m = match_zone(
      &zones,
      &["example.com".into()],
      "app.dev.example.com",
    )
    .unwrap();
    assert_eq!(m.id, "1");
    // "notexample.com" must not match "example.com"
    let err =
      match_zone(&zones, &[], "app.notexample.com").unwrap_err();
    assert_eq!(err.code, "ZoneNotFound");
  }
}
