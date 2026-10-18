//! Independent observations: publication is never treated as loading or reachability.
use super::{adapters::*, periphery};
use komodo_client::entities::{domain::*, komodo_timestamp};
use std::{
  net::{IpAddr, SocketAddr},
  time::Duration,
};

fn condition(
  kind: DomainConditionType,
  status: ConditionStatus,
  reason: &str,
  message: &str,
) -> DomainCondition {
  DomainCondition {
    condition: kind,
    status,
    reason: reason.into(),
    message: message.into(),
    observed_at: komodo_timestamp(),
  }
}
fn unknown(kind: DomainConditionType) -> DomainCondition {
  condition(
    kind,
    ConditionStatus::Unknown,
    "ObservationUnavailable",
    "The controller could not verify this condition from its network",
  )
}

pub async fn observe_readiness(
  config: &DomainConfig,
  ingress: &Ingress,
  id: &str,
) -> Vec<DomainCondition> {
  let origin_required = config.tls.mode == TlsMode::RequireHttps
    && matches!(
      config.dns,
      DomainDns::Managed {
        proxy_mode: ProxyMode::Proxied,
        ..
      }
    )
    && matches!(
      ingress.config.adapter,
      IngressAdapterConfig::TraefikFileV1 { .. }
    );
  let (dns, entrance, public, origin) = tokio::join!(
    tokio::time::timeout(
      Duration::from_secs(12),
      observe_dns(config, ingress)
    ),
    tokio::time::timeout(
      Duration::from_secs(12),
      observe_ingress(config, ingress, id)
    ),
    tokio::time::timeout(
      Duration::from_secs(12),
      observe_public(config)
    ),
    tokio::time::timeout(Duration::from_secs(12), async {
      if origin_required {
        verify_proxied_origin(config, ingress).await
      } else {
        true
      }
    }),
  );
  let mut result =
    vec![
      dns.ok().flatten().unwrap_or_else(|| {
        unknown(DomainConditionType::DnsConfigured)
      }),
      entrance.ok().flatten().unwrap_or_else(|| {
        unknown(DomainConditionType::IngressReady)
      }),
    ];
  match public {
    Ok(Some(conditions)) => result.extend(conditions),
    _ => {
      result.push(if config.tls.mode == TlsMode::HttpOnly {
        condition(
          DomainConditionType::TlsReady,
          ConditionStatus::True,
          "NotRequired",
          "HTTP-only policy does not require public TLS",
        )
      } else {
        unknown(DomainConditionType::TlsReady)
      });
      result.push(unknown(DomainConditionType::Reachable));
    }
  }
  if origin_required && !origin.unwrap_or(false) {
    if let Some(tls) = result
      .iter_mut()
      .find(|c| c.condition == DomainConditionType::TlsReady)
    {
      *tls = condition(
        DomainConditionType::TlsReady,
        ConditionStatus::Unknown,
        "OriginTlsUnverified",
        "Public edge TLS alone cannot establish the Cloudflare-to-origin TLS leg",
      );
    }
  }
  result
}

async fn verify_proxied_origin(
  config: &DomainConfig,
  ingress: &Ingress,
) -> bool {
  let DomainDns::Managed { provider, .. } = &config.dns else {
    return false;
  };
  let Ok(provider) =
    crate::resource::get::<DnsProvider>(provider).await
  else {
    return false;
  };
  let Ok(cf) = DnsDriver::from_provider(&provider) else {
    return false;
  };
  let strict = tokio::time::timeout(
    Duration::from_secs(8),
    cf.strict_origin_tls(),
  )
  .await;
  matches!(strict, Ok(Ok(true)))
    && verify_candidate(config, ingress).await.is_ok()
}

async fn observe_dns(
  config: &DomainConfig,
  ingress: &Ingress,
) -> Option<DomainCondition> {
  if let DomainDns::Managed {
    provider,
    ttl,
    proxy_mode,
  } = &config.dns
  {
    let provider =
      crate::resource::get::<DnsProvider>(provider).await.ok()?;
    let records = DnsDriver::from_provider(&provider)
      .ok()?
      .observe(&config.hostname)
      .await
      .ok()?;
    let target = match &ingress.config.adapter {
      IngressAdapterConfig::TraefikFileV1 { dns_target, .. } => {
        dns_target.clone()
      }
      IngressAdapterConfig::CloudflareTunnelV1 {
        tunnel_id, ..
      } => format!("{tunnel_id}.cfargotunnel.com"),
    };
    let kind = match target.parse::<IpAddr>() {
      Ok(IpAddr::V4(_)) => "A",
      Ok(IpAddr::V6(_)) => "AAAA",
      _ => "CNAME",
    };
    let desired = canonical_address(&target);
    let addresses: Vec<_> = records
      .iter()
      .filter(|r| {
        matches!(r["type"].as_str(), Some("A" | "AAAA" | "CNAME"))
      })
      .collect();
    let proxied = *proxy_mode == ProxyMode::Proxied;
    let matches = addresses.len() == 1
      && addresses[0]["type"] == kind
      && addresses[0]["content"]
        .as_str()
        .is_some_and(|content| canonical_address(content) == desired)
      && addresses[0]["proxied"] == proxied
      && addresses[0]["ttl"].as_u64()
        == Some(if proxied { 1 } else { u64::from(*ttl) });
    return Some(condition(
      DomainConditionType::DnsConfigured,
      if matches {
        ConditionStatus::True
      } else {
        ConditionStatus::False
      },
      if matches {
        "ProviderRecordObserved"
      } else {
        "DnsMismatch"
      },
      "Authoritative provider configuration was compared; propagation is observed separately",
    ));
  }
  let addresses =
    tokio::net::lookup_host((config.hostname.as_str(), 443))
      .await
      .ok()?;
  if addresses.into_iter().any(|a| public_ip(a.ip())) {
    Some(condition(
      DomainConditionType::DnsConfigured,
      ConditionStatus::True,
      "ExternalResolutionObserved",
      "Externally managed DNS resolves; no external record is owned or modified",
    ))
  } else {
    None
  }
}
fn canonical_address(value: &str) -> String {
  value
    .parse::<IpAddr>()
    .map(|ip| ip.to_string())
    .unwrap_or_else(|_| {
      value.trim_end_matches('.').to_ascii_lowercase()
    })
}

async fn observe_ingress(
  config: &DomainConfig,
  ingress: &Ingress,
  id: &str,
) -> Option<DomainCondition> {
  let ready = match &ingress.config.adapter {
    IngressAdapterConfig::TraefikFileV1 { .. } => {
      let client = periphery(ingress).await.ok()?;
      let observation = TraefikFileDriver {
        client: &client,
        ingress_id: &ingress.id,
      }
      .observe(id)
      .await
      .ok()?;
      if observation.load_error.is_some() || observation.rolled_back {
        return Some(condition(
          DomainConditionType::IngressReady,
          ConditionStatus::False,
          "IngressLoadFailed",
          "Traefik rejected the managed configuration",
        ));
      }
      if !observation.exists {
        return Some(condition(
          DomainConditionType::IngressReady,
          ConditionStatus::False,
          "RouteMissing",
          "No managed ingress route was observed",
        ));
      }
      if !observation.load_verified {
        return Some(condition(
          DomainConditionType::IngressReady,
          ConditionStatus::Unknown,
          "LoadUnverified",
          "Route file is published; Traefik loading has not been independently verified",
        ));
      }
      true
    }
    IngressAdapterConfig::CloudflareTunnelV1 {
      account_id,
      tunnel_id,
      secret,
      ..
    } => {
      let cf = cloudflare_from_secret(secret).ok()?;
      let tunnel =
        cf.observe_tunnel(account_id, tunnel_id).await.ok()?;
      let has_route = tunnel["ingress"]
        .as_array()?
        .iter()
        .any(|r| r["hostname"] == config.hostname);
      has_route
        && cf.tunnel_connected(account_id, tunnel_id).await.ok()?
    }
  };
  Some(condition(
    DomainConditionType::IngressReady,
    if ready {
      ConditionStatus::True
    } else {
      ConditionStatus::False
    },
    if ready {
      "IngressObserved"
    } else {
      "IngressNotReady"
    },
    "Ingress route and adapter readiness were independently observed",
  ))
}

/// Reject every non-global address before constructing a request. Pin the approved
/// resolution to the hostname (TLS still validates the original hostname), and
/// disable redirects/proxies so a DNS change cannot turn probes into internal SSRF.
async fn observe_public(
  config: &DomainConfig,
) -> Option<Vec<DomainCondition>> {
  let https = config.tls.mode == TlsMode::RequireHttps;
  let port = if https { 443 } else { 80 };
  let addresses: Vec<SocketAddr> =
    tokio::net::lookup_host((config.hostname.as_str(), port))
      .await
      .ok()?
      .collect();
  if addresses.is_empty()
    || addresses.iter().any(|address| !public_ip(address.ip()))
  {
    return None;
  }
  let client = reqwest::Client::builder()
    .no_proxy()
    .timeout(Duration::from_secs(8))
    .redirect(reqwest::redirect::Policy::none())
    .resolve_to_addrs(&config.hostname, &addresses)
    .build()
    .ok()?;
  let response = client
    .get(format!(
      "{}://{}/",
      if https { "https" } else { "http" },
      config.hostname
    ))
    .send()
    .await
    .ok()?;
  let reachable = response.status().is_success()
    || response.status().is_redirection();
  Some(vec![
    condition(
      DomainConditionType::TlsReady,
      ConditionStatus::True,
      if https {
        "PublicTlsVerified"
      } else {
        "NotRequired"
      },
      if https {
        "Public certificate chain and hostname verified"
      } else {
        "HTTP-only policy does not require public TLS"
      },
    ),
    condition(
      DomainConditionType::Reachable,
      if reachable {
        ConditionStatus::True
      } else {
        ConditionStatus::False
      },
      if reachable {
        "HttpResponseObserved"
      } else {
        "HttpErrorObserved"
      },
      "Public endpoint HTTP response observed; application identity and origin TLS require separate verification",
    ),
  ])
}

fn public_ip(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(ip) => {
      let [a, b, c, _] = ip.octets();
      !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
    }
    IpAddr::V6(ip) => {
      if let Some(mapped) = ip.to_ipv4_mapped() {
        return public_ip(IpAddr::V4(mapped));
      }
      let octets = ip.octets();
      // Conservative global-unicast only, excluding documentation and embedded IPv4 tunnels.
      octets[0] & 0xe0 == 0x20
        && !(octets[0] == 0x20
          && octets[1] == 0x01
          && ((octets[2] == 0x0d && octets[3] == 0xb8)
            || (octets[2] < 2)))
        && !(octets[0] == 0x20 && octets[1] == 0x02)
        && !(octets[0] == 0x3f
          && octets[1] == 0xff
          && octets[2] & 0xf0 == 0)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn probes_reject_internal_metadata_and_reserved_addresses() {
    for address in [
      "127.0.0.1",
      "169.254.169.254",
      "100.100.100.200",
      "10.0.0.1",
      "192.0.2.1",
      "198.18.0.1",
      "::1",
      "::ffff:169.254.169.254",
      "fd00::1",
      "2001:db8::1",
      "2002:7f00:1::",
    ] {
      assert!(!public_ip(address.parse().unwrap()), "{address}");
    }
    for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
      assert!(public_ip(address.parse().unwrap()), "{address}");
    }
  }
}

/// Probe the candidate entrance directly, retaining the Domain hostname for SNI
/// and Host. Looking up the Domain itself here would merely verify the old route.
pub async fn verify_candidate(
  config: &DomainConfig,
  ingress: &Ingress,
) -> anyhow::Result<()> {
  let target = match &ingress.config.adapter {
    IngressAdapterConfig::TraefikFileV1 { dns_target, .. } => {
      dns_target
    }
    IngressAdapterConfig::CloudflareTunnelV1 { .. } => anyhow::bail!(
      "RequiresDowntime: Tunnel cannot be independently verified before DNS cutover"
    ),
  };
  tokio::time::timeout(Duration::from_secs(12),async {
    let https=config.tls.mode==TlsMode::RequireHttps;
    let port=if https{443}else{80};
    let addresses:Vec<SocketAddr>=match target.parse::<IpAddr>() {
      Ok(ip)=>vec![SocketAddr::new(ip,port)],
      Err(_)=>tokio::net::lookup_host((target.as_str(),port)).await.map_err(|_|anyhow::anyhow!("VerificationPending: candidate address could not be resolved"))?.collect(),
    };
    if addresses.is_empty() || addresses.iter().any(|a|!public_ip(a.ip())) {anyhow::bail!("VerificationPending: candidate is not publicly probeable from this controller");}
    let client=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(8)).redirect(reqwest::redirect::Policy::none()).resolve_to_addrs(&config.hostname,&addresses).build()?;
    let response=client.get(format!("{}://{}/",if https{"https"}else{"http"},config.hostname)).send().await.map_err(|_|anyhow::anyhow!("VerificationPending: candidate entrance HTTP or TLS could not be verified"))?;
    if !response.status().is_success(){anyhow::bail!("VerificationPending: candidate entrance did not return a healthy response");}
    Ok(())
  }).await.map_err(|_|anyhow::anyhow!("VerificationPending: candidate entrance probe timed out"))?
}
