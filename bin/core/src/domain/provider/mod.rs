//! Provider abstraction. A provider kind (eg. `cloudflare`) is registered
//! once in [REGISTRY] and may implement [DnsProvider], [IngressProvider], or both.
//!
//! Adding a vendor = one new file + one line in [REGISTRY].

use std::{fmt, future::Future, pin::Pin};

use komodo_client::entities::{
  JsonValue,
  domain::{DnsRecordType, DomainLayer, OwnedObject},
  domain_account::DomainDiscovery,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::error::ProviderError;

mod cloudflare;
mod cloudflare_client;
mod cloudflare_provision;
mod provision;
mod tunnel_merge;

pub use provision::{ConnectorSpec, ProvisionProvider, TunnelHandle};

pub type BoxFut<'a, T> =
  Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

/// Who owns remote objects: the Domain resource.
#[derive(Debug, Clone)]
pub struct Owner {
  /// Domain resource id
  pub id: String,
}

impl Owner {
  /// Marker written on remote objects which support comments.
  pub fn marker(&self) -> String {
    format!("komodo:{}", self.id)
  }
}

// DNS

/// Fully resolved desired DNS record.
#[derive(Debug, Clone, PartialEq)]
pub struct DnsRecordSpec {
  pub record_type: DnsRecordType,
  pub content: String,
  /// 1 = provider automatic
  pub ttl: u32,
  pub proxied: bool,
}

impl DnsRecordSpec {
  pub fn canonical(&self) -> Value {
    dns_canonical(
      self.record_type.as_str(),
      &self.content,
      self.ttl,
      self.proxied,
    )
  }
}

/// The normalized form compared by plan and hashed for drift detection.
/// Providers MUST produce remote records in exactly this shape.
pub fn dns_canonical(
  record_type: &str,
  content: &str,
  ttl: u32,
  proxied: bool,
) -> Value {
  let content = content
    .parse::<std::net::IpAddr>()
    .map(|ip| ip.to_string())
    .unwrap_or_else(|_| {
      content.trim_end_matches('.').to_ascii_lowercase()
    });
  json!({
    "type": record_type,
    "content": content,
    // proxied records always have automatic ttl
    "ttl": if proxied || ttl == 0 { 1 } else { ttl },
    "proxied": proxied,
  })
}

#[derive(Debug, Clone)]
pub struct RemoteRecord {
  /// Provider object id
  pub remote_id: String,
  /// Provider container (eg. zone id), needed to write / delete
  pub scope: String,
  /// See [dns_canonical]
  pub canonical: Value,
  /// Ownership marker found on the record, if the provider has one.
  pub marker: Option<String>,
}

pub trait DnsProvider: Send + Sync {
  /// Local validation of the desired record. No network.
  fn validate(
    &self,
    hostname: &str,
    spec: &DnsRecordSpec,
  ) -> Result<(), ProviderError>;

  /// All address records (A / AAAA / CNAME) named exactly `hostname`.
  fn list<'a>(
    &'a self,
    hostname: &'a str,
  ) -> BoxFut<'a, Vec<RemoteRecord>>;

  /// Create the record, or update `existing`. Writes the ownership marker.
  fn upsert<'a>(
    &'a self,
    hostname: &'a str,
    spec: &'a DnsRecordSpec,
    owner: &'a Owner,
    existing: Option<&'a RemoteRecord>,
  ) -> BoxFut<'a, RemoteRecord>;

  /// Delete a record Komodo created. Must be a no-op if the record
  /// is already gone or no longer carries the owner's marker.
  fn delete<'a>(
    &'a self,
    owned: &'a OwnedObject,
    owner: &'a Owner,
  ) -> BoxFut<'a, ()>;

  fn is_owned(&self, record: &RemoteRecord, owner: &Owner) -> bool {
    record.marker.as_deref() == Some(owner.marker().as_str())
  }
}

// INGRESS

/// A tunnel / entrypoint resolved from account params.
#[derive(Debug, Clone)]
pub struct IngressTarget {
  /// Provider container id. Stored as `OwnedObject.scope`.
  pub id: String,
  /// What DNS should point at to reach this ingress, if anything.
  pub dns_target: Option<DnsTarget>,
}

#[derive(Debug, Clone)]
pub struct DnsTarget {
  pub record_type: DnsRecordType,
  pub content: String,
  pub proxied: bool,
}

/// The Komodo Server a Deployment / Stack runs on.
#[derive(Debug, Clone)]
pub struct ServerRef {
  pub id: String,
  pub name: String,
}

#[derive(Debug, Clone)]
pub struct RemoteRule {
  pub service: String,
  /// The whole rule as returned by the provider (may hold fields
  /// Komodo doesn't manage). Used for ownership digests.
  pub rule: Value,
}

pub trait IngressProvider: Send + Sync {
  /// Local validation of the origin service. No network.
  fn validate(&self, service: &str) -> Result<(), ProviderError>;

  /// Pick the ingress by registered `name`, or by the `server` it is
  /// bound to. Errors when nothing / more than one matches.
  fn select(
    &self,
    name: Option<&str>,
    server: Option<&ServerRef>,
  ) -> Result<IngressTarget, ProviderError>;

  /// The rule for `hostname`, if any. Other hostnames are never read by callers.
  fn get<'a>(
    &'a self,
    target: &'a IngressTarget,
    hostname: &'a str,
  ) -> BoxFut<'a, Option<RemoteRule>>;

  /// Create or update the rule for `hostname`, leaving every other rule
  /// and unknown field untouched.
  fn put<'a>(
    &'a self,
    target: &'a IngressTarget,
    hostname: &'a str,
    service: &'a str,
  ) -> BoxFut<'a, RemoteRule>;

  /// Remove a rule Komodo created. No-op if it is already gone.
  fn remove<'a>(&'a self, owned: &'a OwnedObject) -> BoxFut<'a, ()>;
}

// REGISTRY

/// Credentials of one account. No Debug output of the token.
#[derive(Clone)]
pub struct Credentials {
  pub name: String,
  pub token: String,
  pub params: JsonValue,
}

impl fmt::Debug for Credentials {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("Credentials")
      .field("name", &self.name)
      .field("token", &"***")
      .finish()
  }
}

#[derive(Default)]
pub struct ProviderSet {
  pub dns: Option<Box<dyn DnsProvider>>,
  pub ingress: Option<Box<dyn IngressProvider>>,
  /// Optional: can open a tunnel by itself.
  pub provision: Option<Box<dyn ProvisionProvider>>,
}

pub struct ProviderFactory {
  pub kind: &'static str,
  pub capabilities: &'static [DomainLayer],
  pub params_example: fn() -> JsonValue,
  /// Validate account params. No network.
  pub validate_params: fn(&JsonValue) -> Result<(), ProviderError>,
  pub build: fn(Credentials) -> Result<ProviderSet, ProviderError>,
  /// Optional: list the zones / ingresses a token can see (read only),
  /// so the UI can offer a pick list instead of free text.
  pub discover:
    Option<fn(String) -> BoxFut<'static, DomainDiscovery>>,
}

/// Add new vendors here.
pub static REGISTRY: &[ProviderFactory] = &[cloudflare::FACTORY];

pub fn factory(kind: &str) -> Option<&'static ProviderFactory> {
  REGISTRY.iter().find(|f| f.kind == kind)
}

// DIGEST

/// SHA-256 of canonical (key sorted) JSON.
pub fn digest(value: &Value) -> String {
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
