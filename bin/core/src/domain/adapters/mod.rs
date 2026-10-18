//! Provider boundary. Never include provider response bodies or credentials in errors.
//! Callers must hold persistent binding/shared-ingress leases across writes.
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
pub mod registry;
pub use registry::{DnsDriver, IngressDriver};

use std::{fmt, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterError {
  pub code: String,
  pub message: String,
  pub retryable: bool,
}
impl AdapterError {
  pub fn new(code: &str, message: &str, retryable: bool) -> Self {
    Self {
      code: code.into(),
      message: message.into(),
      retryable,
    }
  }
}
impl fmt::Display for AdapterError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}: {}", self.code, self.message)
  }
}
impl std::error::Error for AdapterError {}
type Result<T> = std::result::Result<T, AdapterError>;
/// Entire read/compare/write cycle is shorter than Core's 120-second lease.
/// A timed-out request has an ambiguous result and MUST be observed before replay.
async fn bounded_write<T>(
  future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
  tokio::time::timeout(Duration::from_secs(60), future)
    .await
    .map_err(|_| {
      AdapterError::new(
        "ProviderTimeout",
        "Operation deadline exceeded; observe before retrying",
        true,
      )
    })?
}

fn drift() -> AdapterError {
  AdapterError::new(
    "Drifted",
    "External configuration changed; create a new plan",
    false,
  )
}
fn conflict() -> AdapterError {
  AdapterError::new(
    "OwnershipConflict",
    "An external object is not owned by this binding",
    false,
  )
}
fn malformed() -> AdapterError {
  AdapterError::new(
    "ProviderResponse",
    "Provider returned an invalid response",
    false,
  )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnedObject {
  pub id: String,
  pub fingerprint: String,
  pub owner: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsIntent {
  pub hostname: String,
  pub record_type: String,
  pub content: String,
  pub ttl: u32,
  pub proxied: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSpec {
  pub binding_id: String,
  pub hostname: String,
  pub backend_url: String,
}

/// SHA-256 of canonical JSON: independent of map insertion order.
pub fn fingerprint(value: &Value) -> String {
  fn canonical(v: &Value) -> Value {
    match v {
      Value::Object(map) => Value::Object(
        map
          .iter()
          .map(|(k, v)| (k.clone(), canonical(v)))
          .collect::<std::collections::BTreeMap<_, _>>()
          .into_iter()
          .collect(),
      ),
      Value::Array(a) => {
        Value::Array(a.iter().map(canonical).collect())
      }
      _ => v.clone(),
    }
  }
  hex::encode(Sha256::digest(canonical(value).to_string().as_bytes()))
}
pub fn dns_canonical_content(value: &str) -> String {
  value
    .parse::<std::net::IpAddr>()
    .map(|ip| ip.to_string())
    .unwrap_or_else(|_| {
      value.trim_end_matches('.').to_ascii_lowercase()
    })
}
pub fn dns_fingerprint(record: &Value) -> String {
  fingerprint(&dns_managed(record))
}
fn dns_managed(record: &Value) -> Value {
  json!({"type":record["type"],"name":record["name"],"content":record["content"],"ttl":record["ttl"],"proxied":record["proxied"],"comment":record["comment"],"settings":record["settings"],"tags":record["tags"]})
}
fn segment(value: &str) -> Result<&str> {
  if value.is_empty()
    || !value
      .bytes()
      .all(|c| c.is_ascii_alphanumeric() || c == b'-')
  {
    return Err(AdapterError::new(
      "InvalidConfiguration",
      "Invalid provider object identifier",
      false,
    ));
  }
  Ok(value)
}

/// No Debug/Serialize implementation: token must never escape the control plane.
pub struct CloudflareClient {
  client: Client,
  token: String,
  #[cfg(test)]
  base_url: Option<String>,
}
impl CloudflareClient {
  pub fn new(secret: &str) -> Result<Self> {
    if secret.is_empty() {
      return Err(AdapterError::new(
        "CredentialMissing",
        "Core secret is empty",
        false,
      ));
    }
    Ok(Self {
      client: Client::builder()
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| malformed())?,
      token: secret.into(),
      #[cfg(test)]
      base_url: None,
    })
  }
  fn endpoint(&self, path: &str) -> String {
    #[cfg(test)]
    if let Some(base) = &self.base_url {
      return format!("{base}{path}");
    }
    format!("https://api.cloudflare.com/client/v4{path}")
  }
  async fn request(
    &self,
    method: Method,
    path: &str,
    body: Option<&Value>,
  ) -> Result<Value> {
    // Only GET is retried: ambiguous mutations must be observed before replay.
    for attempt in 0..3u32 {
      let mut req = self
        .client
        .request(method.clone(), self.endpoint(path))
        .bearer_auth(&self.token);
      if let Some(body) = body {
        req = req.json(body);
      }
      let response = req.send().await.map_err(|_| {
        AdapterError::new(
          "ProviderUnavailable",
          "Provider request failed; observe before retrying",
          true,
        )
      })?;
      let status = response.status();
      if (status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error())
        && method == Method::GET
        && attempt < 2
      {
        let delay = response
          .headers()
          .get("retry-after")
          .and_then(|v| v.to_str().ok())
          .and_then(|v| v.parse::<u64>().ok())
          .unwrap_or(1 << attempt)
          .min(10);
        tokio::time::sleep(Duration::from_secs(delay)).await;
        continue;
      }
      if !status.is_success() {
        return Err(match status.as_u16() {
          401 | 403 => AdapterError::new(
            "PermissionDenied",
            "Provider authorization failed",
            false,
          ),
          404 => AdapterError::new(
            "NotFound",
            "Provider object was not found",
            false,
          ),
          409 => conflict(),
          429 => AdapterError::new(
            "RateLimited",
            "Provider rate limit reached",
            true,
          ),
          500..=599 => AdapterError::new(
            "ProviderUnavailable",
            "Provider is unavailable",
            true,
          ),
          _ => AdapterError::new(
            "ProviderRejected",
            "Provider rejected the request",
            false,
          ),
        });
      }
      let data: Value =
        response.json().await.map_err(|_| malformed())?;
      if data["success"] != true {
        return Err(AdapterError::new(
          "ProviderRejected",
          "Provider rejected the operation",
          false,
        ));
      }
      return Ok(data);
    }
    unreachable!()
  }
  pub async fn validate_zone(
    &self,
    zone: &str,
    account: &str,
  ) -> Result<Value> {
    segment(account)?;
    let result = self
      .request(
        Method::GET,
        &format!("/zones/{}", segment(zone)?),
        None,
      )
      .await?["result"]
      .clone();
    if result["account"]["id"] != account {
      return Err(AdapterError::new(
        "PermissionDenied",
        "Zone belongs to a different account",
        false,
      ));
    }
    if result["status"] != "active" {
      return Err(AdapterError::new(
        "InvalidConfiguration",
        "Zone must be active and authoritative before managing DNS",
        false,
      ));
    }
    Ok(result)
  }
  pub async fn strict_origin_tls(&self, zone: &str) -> Result<bool> {
    let data = self
      .request(
        Method::GET,
        &format!("/zones/{}/settings/ssl", segment(zone)?),
        None,
      )
      .await?;
    Ok(data["result"]["value"] == "strict")
  }
  pub async fn list_dns(
    &self,
    zone: &str,
    hostname: &str,
  ) -> Result<Vec<Value>> {
    let zone = segment(zone)?;
    let mut records = Vec::new();
    for page in 1..=1000 {
      let data = self.request(Method::GET, &format!("/zones/{zone}/dns_records?name={}&page={page}&per_page=100", urlencoding::encode(hostname)), None).await?;
      let items = data["result"].as_array().ok_or_else(malformed)?;
      // Enforce exact host matching locally even if provider filtering changes.
      records.extend(
        items
          .iter()
          .filter(|record| {
            record["name"].as_str().is_some_and(|name| {
              name
                .trim_end_matches('.')
                .eq_ignore_ascii_case(hostname.trim_end_matches('.'))
            })
          })
          .cloned(),
      );
      if items.len() < 100
        || data["result_info"]["total_pages"]
          .as_u64()
          .is_some_and(|n| page >= n)
      {
        return Ok(records);
      }
    }
    Err(AdapterError::new(
      "ProviderLimit",
      "DNS pagination limit exceeded",
      false,
    ))
  }
  pub async fn apply_dns(
    &self,
    zone: &str,
    intent: &DnsIntent,
    owned: Option<&OwnedObject>,
    owner: &str,
  ) -> Result<OwnedObject> {
    bounded_write(self.apply_dns_inner(zone, intent, owned, owner))
      .await
  }
  async fn apply_dns_inner(
    &self,
    zone: &str,
    intent: &DnsIntent,
    owned: Option<&OwnedObject>,
    owner: &str,
  ) -> Result<OwnedObject> {
    if !matches!(intent.record_type.as_str(), "A" | "AAAA" | "CNAME")
    {
      return Err(AdapterError::new(
        "UnsupportedCapability",
        "Unsupported DNS record type",
        false,
      ));
    }
    if intent.hostname.starts_with("*.")
      || intent.hostname.is_empty()
      || intent.hostname.contains(['/', ':'])
      || owner.is_empty()
    {
      return Err(AdapterError::new(
        "InvalidConfiguration",
        "An exact hostname and ownership marker are required",
        false,
      ));
    }
    let valid_content = match intent.record_type.as_str() {
      "A" => intent.content.parse::<std::net::Ipv4Addr>().is_ok(),
      "AAAA" => intent.content.parse::<std::net::Ipv6Addr>().is_ok(),
      "CNAME" => {
        !intent.content.is_empty()
          && intent.content.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.')
          })
      }
      _ => false,
    };
    if !valid_content
      || (intent.ttl != 1 && !(60..=86400).contains(&intent.ttl))
    {
      return Err(AdapterError::new(
        "InvalidConfiguration",
        "Invalid DNS content or TTL",
        false,
      ));
    }
    let records = self.list_dns(zone, &intent.hostname).await?;
    let address: Vec<_> = records
      .iter()
      .filter(|r| {
        matches!(r["type"].as_str(), Some("A" | "AAAA" | "CNAME"))
      })
      .collect();
    let existing = match owned {
      Some(handle) => {
        if handle.owner != owner {
          return Err(conflict());
        }
        let found = address
          .iter()
          .find(|r| r["id"].as_str() == Some(&handle.id))
          .copied()
          .ok_or_else(drift)?;
        if fingerprint(&dns_managed(found)) != handle.fingerprint
          || found["comment"] != owner
        {
          return Err(drift());
        }
        if address.len() != 1 {
          return Err(conflict());
        }
        Some(found)
      }
      None => {
        // Stable marker permits recovery after a successful create whose response was lost.
        if address.is_empty() {
          None
        } else if address.len() == 1 && address[0]["comment"] == owner
        {
          Some(address[0])
        } else {
          return Err(conflict());
        }
      }
    };
    let canonical_content = dns_canonical_content(&intent.content);
    let mut desired = json!({"type": intent.record_type,"name":intent.hostname,"content":canonical_content,"ttl":if intent.proxied {1} else {intent.ttl},"proxied":intent.proxied,"comment":owner});
    if let Some(old) = existing {
      // Preserve vendor fields. Compare desired fields only, fingerprint guards all managed fields.
      if desired
        .as_object()
        .unwrap()
        .iter()
        .all(|(k, v)| old.get(k) == Some(v))
      {
        return Ok(OwnedObject {
          id: old["id"].as_str().ok_or_else(malformed)?.into(),
          fingerprint: fingerprint(&dns_managed(old)),
          owner: owner.into(),
        });
      }
      if owned.is_none() {
        // A recovery marker establishes identity, not permission to overwrite
        // unexpected contents after an ambiguous create.
        return Err(drift());
      }
      for key in ["settings", "tags"] {
        if let Some(v) = old.get(key) {
          desired[key] = v.clone();
        }
      }
    }
    let path = if let Some(old) = existing {
      format!(
        "/zones/{}/dns_records/{}",
        segment(zone)?,
        segment(old["id"].as_str().ok_or_else(malformed)?)?
      )
    } else {
      format!("/zones/{}/dns_records", segment(zone)?)
    };
    let result = self
      .request(
        if existing.is_some() {
          Method::PUT
        } else {
          Method::POST
        },
        &path,
        Some(&desired),
      )
      .await?["result"]
      .clone();
    Ok(OwnedObject {
      id: result["id"].as_str().ok_or_else(malformed)?.into(),
      fingerprint: fingerprint(&dns_managed(&result)),
      owner: owner.into(),
    })
  }
  /// Only invoked for a server-held explicit adoption plan with the exact observed fingerprint.
  pub async fn adopt_dns(
    &self,
    zone: &str,
    object_id: &str,
    expected_fingerprint: &str,
    owner: &str,
  ) -> Result<OwnedObject> {
    bounded_write(self.adopt_dns_inner(
      zone,
      object_id,
      expected_fingerprint,
      owner,
    ))
    .await
  }
  async fn adopt_dns_inner(
    &self,
    zone: &str,
    object_id: &str,
    expected_fingerprint: &str,
    owner: &str,
  ) -> Result<OwnedObject> {
    let path = format!(
      "/zones/{}/dns_records/{}",
      segment(zone)?,
      segment(object_id)?
    );
    let current =
      self.request(Method::GET, &path, None).await?["result"].clone();
    if dns_fingerprint(&current) != expected_fingerprint {
      return Err(drift());
    }
    if !matches!(
      current["type"].as_str(),
      Some("A" | "AAAA" | "CNAME")
    ) || current["name"]
      .as_str()
      .is_some_and(|n| n.starts_with("*."))
    {
      return Err(AdapterError::new(
        "UnsupportedCapability",
        "Only exact address records can be adopted",
        false,
      ));
    }
    // Never silently transfer another Komodo instance's marker.
    if current["comment"]
      .as_str()
      .is_some_and(|s| s.starts_with("komodo:") && s != owner)
    {
      return Err(conflict());
    }
    let result = self
      .request(Method::PATCH, &path, Some(&json!({"comment":owner})))
      .await?["result"]
      .clone();
    Ok(OwnedObject {
      id: object_id.into(),
      fingerprint: dns_fingerprint(&result),
      owner: owner.into(),
    })
  }
  pub async fn remove_dns(
    &self,
    zone: &str,
    owned: &OwnedObject,
  ) -> Result<()> {
    bounded_write(self.remove_dns_inner(zone, owned)).await
  }
  async fn remove_dns_inner(
    &self,
    zone: &str,
    owned: &OwnedObject,
  ) -> Result<()> {
    let path = format!(
      "/zones/{}/dns_records/{}",
      segment(zone)?,
      segment(&owned.id)?
    );
    let current = match self.request(Method::GET, &path, None).await {
      Ok(v) => v["result"].clone(),
      Err(e) if e.code == "NotFound" => return Ok(()),
      Err(e) => return Err(e),
    };
    if current["comment"] != owned.owner
      || fingerprint(&dns_managed(&current)) != owned.fingerprint
    {
      return Err(drift());
    }
    self.request(Method::DELETE, &path, None).await?;
    Ok(())
  }
  pub async fn observe_tunnel(
    &self,
    account: &str,
    tunnel: &str,
  ) -> Result<Value> {
    let result = self
      .request(
        Method::GET,
        &format!(
          "/accounts/{}/cfd_tunnel/{}/configurations",
          segment(account)?,
          segment(tunnel)?
        ),
        None,
      )
      .await?;
    if !result["result"]["config"].is_object() {
      return Err(malformed());
    }
    Ok(result["result"]["config"].clone())
  }
  pub async fn tunnel_connected(
    &self,
    account: &str,
    tunnel: &str,
  ) -> Result<bool> {
    let result = self
      .request(
        Method::GET,
        &format!(
          "/accounts/{}/cfd_tunnel/{}",
          segment(account)?,
          segment(tunnel)?
        ),
        None,
      )
      .await?;
    Ok(result["result"]["status"] == "healthy")
  }
  /// Caller must enforce dedicated single-writer Tunnel and a persistent per-Tunnel lease.
  /// CF has no compare-and-swap here; a final observation detects but cannot close that race.
  pub async fn apply_tunnel_route(
    &self,
    account: &str,
    tunnel: &str,
    expected_digest: &str,
    route: &RouteSpec,
    owned: Option<&OwnedObject>,
  ) -> Result<OwnedObject> {
    bounded_write(self.apply_tunnel_route_inner(
      account,
      tunnel,
      expected_digest,
      route,
      owned,
    ))
    .await
  }
  async fn apply_tunnel_route_inner(
    &self,
    account: &str,
    tunnel: &str,
    expected_digest: &str,
    route: &RouteSpec,
    owned: Option<&OwnedObject>,
  ) -> Result<OwnedObject> {
    let config = self.observe_tunnel(account, tunnel).await?;
    if fingerprint(&config) != expected_digest {
      return Err(drift());
    }
    let next = merge_tunnel(&config, route, owned, false)?;
    let rule = next["ingress"]
      .as_array()
      .ok_or_else(malformed)?
      .iter()
      .find(|r| r["hostname"] == route.hostname)
      .ok_or_else(malformed)?
      .clone();
    if next != config {
      if fingerprint(&self.observe_tunnel(account, tunnel).await?)
        != expected_digest
      {
        return Err(drift());
      }
      self
        .request(
          Method::PUT,
          &format!(
            "/accounts/{}/cfd_tunnel/{}/configurations",
            segment(account)?,
            segment(tunnel)?
          ),
          Some(&json!({"config":next})),
        )
        .await?;
    }
    Ok(OwnedObject {
      id: route.hostname.clone(),
      fingerprint: fingerprint(&rule),
      owner: route.binding_id.clone(),
    })
  }
  /// Explicit plan-only ownership transfer. Tunnel rules lack provider IDs;
  /// the exact hostname is the stable handle. This method does not write CF.
  pub async fn adopt_tunnel_route(
    &self,
    account: &str,
    tunnel: &str,
    expected_digest: &str,
    hostname: &str,
    expected_rule_digest: &str,
    binding_id: &str,
  ) -> Result<OwnedObject> {
    let config = self.observe_tunnel(account, tunnel).await?;
    if fingerprint(&config) != expected_digest {
      return Err(drift());
    }
    adopt_tunnel_rule(
      &config,
      hostname,
      expected_rule_digest,
      binding_id,
    )
  }
  pub async fn remove_tunnel_route(
    &self,
    account: &str,
    tunnel: &str,
    expected_digest: &str,
    owned: &OwnedObject,
  ) -> Result<()> {
    bounded_write(self.remove_tunnel_route_inner(
      account,
      tunnel,
      expected_digest,
      owned,
    ))
    .await
  }
  async fn remove_tunnel_route_inner(
    &self,
    account: &str,
    tunnel: &str,
    expected_digest: &str,
    owned: &OwnedObject,
  ) -> Result<()> {
    let config = self.observe_tunnel(account, tunnel).await?;
    if fingerprint(&config) != expected_digest {
      return Err(drift());
    }
    let route = RouteSpec {
      binding_id: owned.owner.clone(),
      hostname: owned.id.clone(),
      backend_url: String::new(),
    };
    let next = merge_tunnel(&config, &route, Some(owned), true)?;
    if next != config {
      if fingerprint(&self.observe_tunnel(account, tunnel).await?)
        != expected_digest
      {
        return Err(drift());
      }
      self
        .request(
          Method::PUT,
          &format!(
            "/accounts/{}/cfd_tunnel/{}/configurations",
            segment(account)?,
            segment(tunnel)?
          ),
          Some(&json!({"config":next})),
        )
        .await?;
    }
    Ok(())
  }
}

/// Pure validation for the exact server-observed adoption intent.
pub fn adopt_tunnel_rule(
  config: &Value,
  hostname: &str,
  expected_rule_digest: &str,
  binding_id: &str,
) -> Result<OwnedObject> {
  if hostname.is_empty()
    || hostname.starts_with("*.")
    || binding_id.is_empty()
  {
    return Err(conflict());
  }
  let rules = config["ingress"].as_array().ok_or_else(malformed)?;
  let matches: Vec<_> = rules
    .iter()
    .filter(|rule| rule["hostname"] == hostname)
    .collect();
  if matches.len() != 1 {
    return Err(conflict());
  }
  let observed = fingerprint(matches[0]);
  if observed != expected_rule_digest {
    return Err(drift());
  }
  Ok(OwnedObject {
    id: hostname.into(),
    fingerprint: observed,
    owner: binding_id.into(),
  })
}

pub fn merge_tunnel(
  config: &Value,
  route: &RouteSpec,
  owned: Option<&OwnedObject>,
  remove: bool,
) -> Result<Value> {
  if !remove && route.backend_url.starts_with("https://") {
    let per_route = config["ingress"]
      .as_array()
      .and_then(|rules| {
        rules.iter().find(|rule| rule["hostname"] == route.hostname)
      })
      .and_then(|rule| rule.get("originRequest"))
      .and_then(|origin| origin.get("noTLSVerify"));
    let insecure = per_route
      .or_else(|| {
        config
          .get("originRequest")
          .and_then(|origin| origin.get("noTLSVerify"))
      })
      .and_then(Value::as_bool)
      .unwrap_or(false);
    if insecure {
      return Err(AdapterError::new(
        "UnsupportedCapability",
        "HTTPS backend certificate verification must remain enabled",
        false,
      ));
    }
  }
  let mut next = config.clone();
  let rules = next["ingress"].as_array_mut().ok_or_else(malformed)?;
  if rules.is_empty()
    || rules.last().unwrap().get("hostname").is_some()
    || rules.last().unwrap().get("path").is_some()
  {
    return Err(AdapterError::new(
      "InvalidConfiguration",
      "Tunnel must end with a catch-all rule",
      false,
    ));
  }
  // An earlier wildcard can capture this host. Dedicated configuration does not
  // authorize silently shadowing a manually maintained rule.
  if rules.iter().any(|r| {
    r["hostname"].as_str().is_some_and(|h| {
      h == "*"
        || (h.starts_with("*.") && route.hostname.ends_with(&h[1..]))
        || (h.contains('*') && !h.starts_with("*."))
    })
  }) {
    return Err(conflict());
  }
  let matching: Vec<_> = rules
    .iter()
    .enumerate()
    .filter(|(_, r)| r["hostname"] == route.hostname)
    .map(|(i, _)| i)
    .collect();
  if matching.len() > 1 {
    return Err(conflict());
  }
  if let Some(&index) = matching.first() {
    let owned = owned.ok_or_else(conflict)?;
    if owned.id != route.hostname
      || owned.owner != route.binding_id
      || fingerprint(&rules[index]) != owned.fingerprint
    {
      return Err(drift());
    }
    if remove {
      rules.remove(index);
    } else {
      rules[index]["service"] = json!(route.backend_url);
    }
  } else if remove {
    return Ok(next);
  } else {
    if owned.is_some() {
      return Err(drift());
    }
    rules.insert(
      rules.len() - 1,
      json!({"hostname":route.hostname,"service":route.backend_url}),
    );
  }
  Ok(next)
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn tunnel_preserves_unknown_fields_and_catchall() {
    let cfg = json!({"originRequest":{"connectTimeout":10},"future":123,"ingress":[{"hostname":"manual.test","service":"http://manual:80","future":true},{"service":"http_status:404"}]});
    let route = RouteSpec {
      binding_id: "binding".into(),
      hostname: "new.test".into(),
      backend_url: "http://app:80".into(),
    };
    let next = merge_tunnel(&cfg, &route, None, false).unwrap();
    assert_eq!(next["future"], 123);
    assert_eq!(next["ingress"][0], cfg["ingress"][0]);
    assert_eq!(next["ingress"][2], cfg["ingress"][1]);
    assert_eq!(
      merge_tunnel(&next, &route, None, false).unwrap_err().code,
      "OwnershipConflict"
    );
    let handle = OwnedObject {
      id: route.hostname.clone(),
      owner: route.binding_id.clone(),
      fingerprint: fingerprint(&next["ingress"][1]),
    };
    assert_eq!(
      merge_tunnel(&next, &route, Some(&handle), false).unwrap(),
      next
    );
    assert_eq!(
      merge_tunnel(&next, &route, Some(&handle), true).unwrap(),
      cfg
    );
    let mut drifted = next;
    drifted["ingress"][1]["service"] = json!("http://manual:90");
    assert_eq!(
      merge_tunnel(&drifted, &route, Some(&handle), true)
        .unwrap_err()
        .code,
      "Drifted"
    );
  }
  #[test]
  fn invalid_identifiers_cannot_change_api_path() {
    for bad in ["../other", "a/b", "x?secret=foo", ""] {
      assert!(segment(bad).is_err());
    }
  }
  #[test]
  fn reject_missing_catchall() {
    let r = RouteSpec {
      binding_id: "a".into(),
      hostname: "a.test".into(),
      backend_url: "http://a".into(),
    };
    assert!(
      merge_tunnel(&json!({"ingress":[]}), &r, None, false).is_err()
    );
  }
}

/// Resolve only administrator-provisioned Core secrets; no general Variables lookup.
pub fn cloudflare_from_secret(
  reference: &komodo_client::entities::domain::SecretRef,
) -> Result<CloudflareClient> {
  let value = crate::config::core_config()
    .secrets
    .get(&reference.name)
    .ok_or_else(|| {
      AdapterError::new(
        "CredentialMissing",
        "Referenced Core secret is unavailable",
        false,
      )
    })?;
  CloudflareClient::new(value)
}

/// Authenticated, structured RPC only. There is deliberately no shell/path fallback.
pub struct TraefikFileDriver<'a> {
  pub client: &'a crate::periphery::PeripheryClient,
  pub ingress_id: &'a str,
}
impl TraefikFileDriver<'_> {
  pub async fn validate(&self) -> Result<()> {
    let capabilities = self
      .client
      .request(
        periphery_client::api::domain::GetDomainIngressCapabilities {
          ingress_id: self.ingress_id.into(),
        },
      )
      .await
      .map_err(periphery_error)?;
    if capabilities.rpc_version != 1
      || !capabilities.enabled
      || !capabilities.dedicated
    {
      return Err(AdapterError::new(
        "UnsupportedCapability",
        "A dedicated enabled Traefik entrance with domain RPC v1 is required; upgrade Periphery if necessary",
        false,
      ));
    }
    Ok(())
  }
  pub async fn observe(
    &self,
    domain_id: &str,
  ) -> Result<periphery_client::api::domain::DomainRouteObservation>
  {
    self.validate().await?;
    self
      .client
      .request(periphery_client::api::domain::ObserveDomainRoute {
        ingress_id: self.ingress_id.into(),
        domain_id: domain_id.into(),
      })
      .await
      .map_err(periphery_error)
  }
  pub async fn apply(
    &self,
    domain_id: &str,
    fencing_token: u64,
    expected_fingerprint: Option<String>,
    route: periphery_client::api::domain::DomainRoute,
  ) -> Result<periphery_client::api::domain::DomainRouteObservation>
  {
    bounded_write(self.apply_inner(
      domain_id,
      fencing_token,
      expected_fingerprint,
      route,
    ))
    .await
  }
  async fn apply_inner(
    &self,
    domain_id: &str,
    fencing_token: u64,
    expected_fingerprint: Option<String>,
    route: periphery_client::api::domain::DomainRoute,
  ) -> Result<periphery_client::api::domain::DomainRouteObservation>
  {
    self.validate().await?;
    let observation = self
      .client
      .request(periphery_client::api::domain::ApplyDomainRoute {
        expires_at_ms: request_expiry(),
        ingress_id: self.ingress_id.into(),
        domain_id: domain_id.into(),
        fencing_token,
        expected_fingerprint,
        route,
      })
      .await
      .map_err(periphery_error)?;
    if observation.load_error.is_some() || observation.rolled_back {
      return Err(AdapterError::new(
        "IngressLoadFailed",
        "Traefik rejected the managed route; previous valid configuration was retained where possible",
        false,
      ));
    }
    Ok(observation)
  }
  pub async fn remove_owned(
    &self,
    domain_id: &str,
    fencing_token: u64,
    expected_fingerprint: Option<String>,
  ) -> Result<periphery_client::api::domain::DomainRouteObservation>
  {
    bounded_write(self.remove_owned_inner(
      domain_id,
      fencing_token,
      expected_fingerprint,
    ))
    .await
  }
  async fn remove_owned_inner(
    &self,
    domain_id: &str,
    fencing_token: u64,
    expected_fingerprint: Option<String>,
  ) -> Result<periphery_client::api::domain::DomainRouteObservation>
  {
    self.validate().await?;
    self
      .client
      .request(periphery_client::api::domain::RemoveDomainRoute {
        expires_at_ms: request_expiry(),
        ingress_id: self.ingress_id.into(),
        domain_id: domain_id.into(),
        fencing_token,
        expected_fingerprint,
      })
      .await
      .map_err(periphery_error)
  }
}
fn periphery_error(error: anyhow::Error) -> AdapterError {
  // Inspect known codes, never forward arbitrary remote messages or file contents.
  let message = error.to_string();
  if message.contains("Drifted") {
    return drift();
  }
  if message.contains("Fenced") || message.contains("fencing") {
    return AdapterError::new(
      "LeaseLost",
      "Periphery rejected an expired writer",
      false,
    );
  }
  AdapterError::new(
    "IngressUnavailable",
    "Periphery domain RPC failed; check connection and RPC version",
    true,
  )
}

#[cfg(test)]
mod fingerprint_tests {
  use super::*;
  #[test]
  fn canonical_dns_content_is_stable() {
    assert_eq!(
      dns_canonical_content("2001:0db8:0000::0001"),
      "2001:db8::1"
    );
    assert_eq!(
      dns_canonical_content("Origin.EXAMPLE.COM."),
      "origin.example.com"
    );
    assert_eq!(dns_canonical_content("192.0.2.1"), "192.0.2.1");
  }

  #[test]
  fn dns_fingerprint_ignores_timestamps_but_detects_manual_changes() {
    let original = json!({"id":"record","type":"A","name":"a.test","content":"192.0.2.1","ttl":60,"proxied":false,"comment":"komodo:instance:binding","tags":[],"modified_on":"yesterday"});
    let mut changed = original.clone();
    changed["modified_on"] = json!("today");
    assert_eq!(dns_fingerprint(&original), dns_fingerprint(&changed));
    changed["content"] = json!("192.0.2.2");
    assert_ne!(dns_fingerprint(&original), dns_fingerprint(&changed));
    assert_eq!(
      fingerprint(&json!({"z":1,"a":{"b":2,"a":1}})),
      fingerprint(&json!({"a":{"a":1,"b":2},"z":1}))
    );
  }
  #[test]
  fn tunnel_does_not_shadow_manual_wildcard() {
    let config = json!({"ingress":[{"hostname":"*.example.com","service":"http://manual"},{"service":"http_status:404"}]});
    let route = RouteSpec {
      binding_id: "id".into(),
      hostname: "app.example.com".into(),
      backend_url: "http://app".into(),
    };
    assert_eq!(
      merge_tunnel(&config, &route, None, false).unwrap_err().code,
      "OwnershipConflict"
    );
  }
  #[test]
  fn tunnel_update_preserves_rule_extensions() {
    let config = json!({"ingress":[{"hostname":"app.test","service":"http://old","originRequest":{"httpHostHeader":"preserve.test"}},{"service":"http_status:404"}]});
    let route = RouteSpec {
      binding_id: "id".into(),
      hostname: "app.test".into(),
      backend_url: "http://new".into(),
    };
    let owned = OwnedObject {
      id: "app.test".into(),
      owner: "id".into(),
      fingerprint: fingerprint(&config["ingress"][0]),
    };
    let next =
      merge_tunnel(&config, &route, Some(&owned), false).unwrap();
    assert_eq!(
      next["ingress"][0]["originRequest"],
      config["ingress"][0]["originRequest"]
    );
    assert_eq!(next["ingress"][0]["service"], "http://new");
  }
}

fn request_expiry() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis()
    .min(u64::MAX as u128) as u64
    + 55_000
}

#[cfg(test)]
mod adoption_tests {
  use super::*;
  #[test]
  fn tunnel_adoption_is_exact_and_nonmutating() {
    let config = json!({"ingress":[{"hostname":"app.test","service":"http://manual","originRequest":{"connectTimeout":8}},{"service":"http_status:404"}]});
    let before = config.clone();
    let rule_digest = fingerprint(&config["ingress"][0]);
    let handle =
      adopt_tunnel_rule(&config, "app.test", &rule_digest, "binding")
        .unwrap();
    assert_eq!(config, before);
    assert_eq!(handle.id, "app.test");
    assert_eq!(handle.owner, "binding");
    assert!(
      adopt_tunnel_rule(&config, "app.test", "stale", "binding")
        .is_err()
    );
    assert!(
      adopt_tunnel_rule(
        &config,
        "other.test",
        &rule_digest,
        "binding"
      )
      .is_err()
    );
    assert!(
      adopt_tunnel_rule(&config, "*.test", &rule_digest, "binding")
        .is_err()
    );
    let route = RouteSpec {
      binding_id: "binding".into(),
      hostname: "app.test".into(),
      backend_url: "http://managed".into(),
    };
    let next =
      merge_tunnel(&config, &route, Some(&handle), false).unwrap();
    assert_eq!(
      next["ingress"][0]["originRequest"],
      config["ingress"][0]["originRequest"]
    );
  }
}

#[cfg(test)]
mod tls_tests {
  use super::*;
  #[test]
  fn tunnel_does_not_silently_disable_backend_tls_verification() {
    let config = json!({"originRequest":{"noTLSVerify":true},"ingress":[{"service":"http_status:404"}]});
    let route = RouteSpec {
      binding_id: "id".into(),
      hostname: "a.test".into(),
      backend_url: "https://backend:443".into(),
    };
    assert_eq!(
      merge_tunnel(&config, &route, None, false).unwrap_err().code,
      "UnsupportedCapability"
    );
  }
}

#[cfg(test)]
mod http_tests;
