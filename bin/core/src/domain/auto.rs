//! Derives a complete Domain config from "this hostname should reach
//! this service". Vendor neutral: it only talks to the provider traits
//! (`IngressProvider::select`) and account zone lists.
//!
//! Rules:
//! 1. The target's Server has a tunnel bound (any account) -> Tunnel:
//!    rule `hostname -> http://localhost:<port>` (assumes the tunnel agent
//!    runs in host network mode on that Server) + DNS to the tunnel.
//! 2. Otherwise -> DNS: A / AAAA to the Server address (CNAME for a name).
//!    A private address is flagged as LAN only.
//! 3. Neither can be decided -> an actionable error.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use komodo_client::entities::{
  ResourceTarget,
  domain::{
    DnsBinding, DnsRecordType, DomainConfig, DomainMode,
    IngressBinding,
  },
};

use super::{
  error::{DomainError, code},
  provider::{DnsProvider, IngressProvider, ServerRef},
};

/// One account as seen by the derivation. No secrets.
pub struct AccountCtx {
  pub name: String,
  pub dns: Option<Box<dyn DnsProvider>>,
  pub ingress: Option<Box<dyn IngressProvider>>,
  /// Zones this account may use. Empty = unknown.
  pub zones: Vec<String>,
}

pub struct ServerInfo {
  pub server: ServerRef,
  /// Host part of `external_address` (or `address`).
  pub host: Option<String>,
}

pub struct DeriveInput<'a> {
  pub hostname: &'a str,
  pub target: ResourceTarget,
  pub service: String,
  pub port: u16,
  pub mode: DomainMode,
  pub server: Option<ServerInfo>,
  pub accounts: &'a [AccountCtx],
}

pub struct Derived {
  pub config: DomainConfig,
  /// `Tunnel` or `Dns`.
  pub mode: DomainMode,
  pub warnings: Vec<String>,
}

const NO_TUNNEL_HINT: &str = "This server has no public connection yet. Make a service public from the Services page to set one up.";

fn zone_matches(hostname: &str, zone: &str) -> bool {
  hostname == zone || hostname.ends_with(&format!(".{zone}"))
}

/// Host part of an address like `wss://1.2.3.4:8120`, `host:8120`, `1.2.3.4`.
pub fn host_of(address: &str) -> Option<String> {
  let address = address.trim();
  if address.is_empty() {
    return None;
  }
  let rest =
    address.split_once("://").map(|(_, r)| r).unwrap_or(address);
  let authority = rest.split(['/', '?']).next().unwrap_or(rest);
  let authority = authority
    .rsplit_once('@')
    .map(|(_, h)| h)
    .unwrap_or(authority);
  let host = if let Some(v6) = authority.strip_prefix('[') {
    v6.split(']').next().unwrap_or(v6)
  } else if authority.matches(':').count() == 1 {
    authority.split(':').next().unwrap_or(authority)
  } else {
    authority
  };
  (!host.is_empty()).then(|| host.to_string())
}

fn private_v4(ip: Ipv4Addr) -> bool {
  let o = ip.octets();
  ip.is_private()
    || ip.is_link_local()
    || (o[0] == 100 && (64..128).contains(&o[1]))
}

fn private_v6(ip: Ipv6Addr) -> bool {
  let s = ip.segments()[0];
  (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
}

/// Whether the address is only reachable from a local network.
pub fn is_private(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => private_v4(v4),
    IpAddr::V6(v6) => private_v6(v6),
  }
}

fn invalid(msg: impl Into<String>) -> DomainError {
  DomainError::invalid(msg)
}

pub fn derive(input: &DeriveInput) -> Result<Derived, DomainError> {
  if input.port == 0 {
    return Err(invalid(
      "Could not detect which port to use. Pick the port to publish.",
    ));
  }
  let server = input.server.as_ref();

  // Tunnel candidates: (account index)
  let mut tunnel_accounts = Vec::new();
  if input.mode != DomainMode::Dns
    && let Some(server) = server
  {
    for (i, account) in input.accounts.iter().enumerate() {
      let Some(ingress) = &account.ingress else {
        continue;
      };
      match ingress.select(None, Some(&server.server)) {
        Ok(_) => tunnel_accounts.push(i),
        Err(e) if e.code == code::TUNNEL_AMBIGUOUS => {
          return Err(invalid(format!(
            "Server '{}' is linked to several tunnels of account '{}'. Keep only one in the account settings.",
            server.server.name, account.name
          )));
        }
        Err(_) => {}
      }
    }
  }
  if tunnel_accounts.len() > 1 {
    return Err(invalid(
      "Server is linked to tunnels of several accounts. Keep only one in the account settings.",
    ));
  }
  let tunnel = tunnel_accounts.first().copied();

  if input.mode == DomainMode::Tunnel && tunnel.is_none() {
    return Err(invalid(NO_TUNNEL_HINT));
  }

  // DNS account: whose zone covers the hostname.
  let dns_accounts: Vec<usize> = input
    .accounts
    .iter()
    .enumerate()
    .filter(|(_, a)| a.dns.is_some())
    .map(|(i, _)| i)
    .collect();
  let matched: Vec<usize> = dns_accounts
    .iter()
    .copied()
    .filter(|i| {
      input.accounts[*i]
        .zones
        .iter()
        .any(|z| zone_matches(input.hostname, z))
    })
    .collect();
  let dns_idx = match (matched.as_slice(), dns_accounts.as_slice()) {
    ([], []) => {
      return Err(invalid(
        "No account can manage DNS yet. Add a Cloudflare account in Settings -> Domains first.",
      ));
    }
    ([], [only]) if input.accounts[*only].zones.is_empty() => *only,
    ([], _) => {
      return Err(invalid(format!(
        "No account manages the domain of '{}'. Check the name, or allow that domain in the account settings.",
        input.hostname
      )));
    }
    (m, _) => match tunnel.filter(|t| m.contains(t)) {
      Some(t) => t,
      None => m[0],
    },
  };
  let dns_account = input.accounts[dns_idx].name.clone();

  let target = Some(input.target.clone());
  let base = DomainConfig {
    hostname: input.hostname.to_string(),
    target,
    service: input.service.clone(),
    port: input.port,
    ..Default::default()
  };

  if let Some(t) = tunnel {
    let config = DomainConfig {
      dns: DnsBinding {
        account: dns_account,
        ..Default::default()
      },
      ingress: IngressBinding {
        account: input.accounts[t].name.clone(),
        service: format!("http://localhost:{}", input.port),
        ..Default::default()
      },
      ..base
    };
    return Ok(Derived {
      config,
      mode: DomainMode::Tunnel,
      warnings: Vec::new(),
    });
  }

  // DNS mode
  let Some(host) = server.and_then(|s| s.host.clone()) else {
    return Err(invalid(match server {
      Some(s) => format!(
        "Server '{}' has no public address and no public connection. Set its External Address in the server settings, or make a service public from the Services page.",
        s.server.name
      ),
      None => "This service is not tied to a single server (Swarm is not supported yet)."
        .to_string(),
    }));
  };
  let mut warnings = Vec::new();
  let (record_type, content) = match host.parse::<IpAddr>() {
    Ok(ip) => {
      if ip.is_loopback() || ip.is_unspecified() {
        return Err(invalid(format!(
          "Server address '{host}' is a loopback address and cannot be a DNS target. Set the server's External Address in its settings."
        )));
      }
      if is_private(ip) {
        warnings.push(format!(
          "The target {ip} is a private address, so this name only works on your local network. Make the service public from the Services page to reach it from the internet."
        ));
      }
      (
        if ip.is_ipv4() {
          DnsRecordType::A
        } else {
          DnsRecordType::AAAA
        },
        ip.to_string(),
      )
    }
    Err(_) => (DnsRecordType::CNAME, host.to_ascii_lowercase()),
  };
  Ok(Derived {
    config: DomainConfig {
      dns: DnsBinding {
        account: dns_account,
        record_type,
        content,
        proxied: Some(false),
        ..Default::default()
      },
      ..base
    },
    mode: DomainMode::Dns,
    warnings,
  })
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use crate::domain::provider::{Credentials, factory};

  use super::*;

  fn account(tunnel_server: Option<&str>) -> AccountCtx {
    let tunnels = tunnel_server
      .map(|s| {
        json!([{
          "name": "home", "account_id": "a1", "tunnel_id": "t1",
          "server": s
        }])
      })
      .unwrap_or(json!([]));
    let set = (factory("cloudflare").unwrap().build)(Credentials {
      name: "cf".into(),
      token: "x".into(),
      params: json!({ "tunnels": tunnels }),
    })
    .unwrap();
    AccountCtx {
      name: "cf".into(),
      dns: set.dns,
      ingress: set.ingress,
      zones: vec!["example.com".into()],
    }
  }

  fn input<'a>(
    accounts: &'a [AccountCtx],
    host: Option<&str>,
  ) -> DeriveInput<'a> {
    DeriveInput {
      hostname: "app.example.com",
      target: ResourceTarget::Deployment("d1".into()),
      service: String::new(),
      port: 8080,
      mode: DomainMode::Auto,
      server: Some(ServerInfo {
        server: ServerRef {
          id: "s1".into(),
          name: "home".into(),
        },
        host: host.map(String::from),
      }),
      accounts,
    }
  }

  #[test]
  fn tunnel_bound_server_uses_tunnel() {
    let accounts = [account(Some("home"))];
    let d = derive(&input(&accounts, Some("192.168.1.2"))).unwrap();
    assert_eq!(d.mode, DomainMode::Tunnel);
    assert_eq!(d.config.ingress.account, "cf");
    assert_eq!(d.config.ingress.service, "http://localhost:8080");
    assert_eq!(d.config.dns.account, "cf");
    assert!(d.config.dns.content.is_empty());
    assert!(d.warnings.is_empty());
  }

  #[test]
  fn no_tunnel_private_ip_uses_dns_with_lan_warning() {
    let accounts = [account(None)];
    let d = derive(&input(
      &accounts,
      Some("wss://192.168.31.251:8120")
        .and_then(host_of)
        .as_deref(),
    ))
    .unwrap();
    assert_eq!(d.mode, DomainMode::Dns);
    assert_eq!(d.config.dns.record_type, DnsRecordType::A);
    assert_eq!(d.config.dns.content, "192.168.31.251");
    assert!(d.warnings[0].contains("local network"));
    assert!(d.config.ingress.account.is_empty());
  }

  #[test]
  fn no_tunnel_no_address_is_actionable() {
    let accounts = [account(None)];
    let e = derive(&input(&accounts, None)).err().unwrap();
    assert!(e.message.contains("External Address"));
  }
}
