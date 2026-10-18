use crate::config::core_config;
use anyhow::{Context, bail};
use komodo_client::entities::{
  deployment::Deployment,
  domain::*,
  permission::{
    PermissionLevel, PermissionLevelAndSpecifics, SpecificPermission,
  },
  stack::Stack,
  user::User,
};

pub fn enabled() -> anyhow::Result<()> {
  if !core_config().domain_management_enabled {
    bail!("FeatureDisabled: domain management is disabled")
  }
  if core_config().domain_instance_id.is_empty() {
    bail!(
      "OwnershipUnconfirmed: configure domain_instance_id explicitly before managing external objects"
    )
  }
  Ok(())
}
pub fn hostname(input: &str) -> anyhow::Result<String> {
  let value = input.strip_suffix('.').unwrap_or(input);
  if value.is_empty()
    || value.contains(['/', ':', '@', '*', '?', '#'])
    || value.trim() != value
  {
    bail!("InvalidHostname: an exact hostname is required")
  }
  let result =
    match url::Host::parse(value).context("InvalidHostname")? {
      url::Host::Domain(value) => value.to_ascii_lowercase(),
      _ => bail!("InvalidHostname: IP literals are not hostnames"),
    };
  if result.len() > 253
    || !result.contains('.')
    || result.split('.').any(|label| {
      label.is_empty()
        || label.len() > 63
        || label.starts_with('-')
        || label.ends_with('-')
        || !label
          .chars()
          .all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
  {
    bail!("InvalidHostname: invalid DNS label")
  }
  Ok(result)
}
pub fn in_scope(host: &str, suffixes: &[String]) -> bool {
  suffixes
    .iter()
    .filter_map(|s| hostname(s).ok())
    .any(|s| host == s || host.ends_with(&format!(".{s}")))
}
pub fn attach(
  specific: SpecificPermission,
) -> PermissionLevelAndSpecifics {
  PermissionLevelAndSpecifics {
    level: PermissionLevel::Read,
    specific: [specific].into_iter().collect(),
  }
}
pub fn provider(config: &DnsProviderConfig) -> anyhow::Result<()> {
  if config.adapter != "CloudflareDnsV1" || config.schema_version != 1
  {
    bail!("UnsupportedCapability: unknown DNS adapter or schema")
  }
  if config.zone_id.is_empty()
    || config.account_id.is_empty()
    || config.secret.name.is_empty()
    || config.allowed_hostname_suffixes.is_empty()
  {
    bail!(
      "InvalidConfig: DNS identity, credential reference and hostname scope required"
    )
  }
  Ok(())
}
pub fn ingress(config: &IngressConfig) -> anyhow::Result<()> {
  if config.schema_version != 1 {
    bail!("UnsupportedCapability: unknown ingress schema")
  }
  if config.allowed_hostname_suffixes.is_empty() {
    bail!("InvalidConfig: ingress hostname scope is required")
  }
  match &config.adapter {
    IngressAdapterConfig::TraefikFileV1 {
      server,
      dedicated,
      dns_target,
      ..
    } => {
      if !dedicated || server.is_empty() || dns_target.is_empty() {
        bail!(
          "InvalidConfig: dedicated Traefik server and DNS target required"
        )
      }
    }
    IngressAdapterConfig::CloudflareTunnelV1 {
      account_id,
      tunnel_id,
      secret,
      dedicated,
    } => {
      if !dedicated
        || account_id.is_empty()
        || tunnel_id.is_empty()
        || secret.name.is_empty()
      {
        bail!(
          "InvalidConfig: dedicated remote Tunnel identity and credential reference required"
        )
      }
    }
  }
  Ok(())
}
pub async fn resolve(
  config: &mut DomainConfig,
  user: &User,
) -> anyhow::Result<(Ingress, Option<DnsProvider>, String)> {
  resolve_inner(config, user, true).await
}
pub async fn validate_desired(
  config: &mut DomainConfig,
  user: &User,
) -> anyhow::Result<()> {
  resolve_inner(config, user, false).await.map(|_| ())
}
async fn resolve_inner(
  config: &mut DomainConfig,
  user: &User,
  live: bool,
) -> anyhow::Result<(Ingress, Option<DnsProvider>, String)> {
  enabled()?;
  config.hostname = hostname(&config.hostname)?;
  let ingress_resource = crate::domain::authorized::<Ingress>(
    &config.ingress,
    user,
    attach(SpecificPermission::Attach),
  )
  .await?;
  ingress(&ingress_resource.config)?;
  config.ingress = ingress_resource.id.clone();
  if !in_scope(
    &config.hostname,
    &ingress_resource.config.allowed_hostname_suffixes,
  ) {
    bail!("Forbidden: hostname outside ingress scope")
  }
  let provider = if let DomainDns::Managed {
    provider,
    ttl,
    proxy_mode,
  } = &mut config.dns
  {
    let resource = crate::domain::authorized::<DnsProvider>(
      provider,
      user,
      attach(SpecificPermission::Attach),
    )
    .await?;
    self::provider(&resource.config)?;
    if !in_scope(
      &config.hostname,
      &resource.config.allowed_hostname_suffixes,
    ) {
      bail!("Forbidden: hostname outside DNS scope")
    }
    if *ttl != 1 && !(60..=86400).contains(ttl) {
      bail!(
        "InvalidConfig: DNS TTL must be automatic (1) or 60..86400"
      )
    }
    if *proxy_mode == ProxyMode::Proxied && *ttl != 1 {
      bail!(
        "UnsupportedCapability: proxied DNS requires automatic TTL"
      )
    }
    *provider = resource.id.clone();
    Some(resource)
  } else {
    None
  };
  if !ingress_resource.config.tls_profiles.iter().any(|p| {
    p.name == config.tls.profile && p.mode == config.tls.mode
  }) {
    bail!(
      "UnsupportedCapability: TLS profile does not support requested mode"
    )
  }
  let backend = match &mut config.target {
    DomainTarget::Url { url } => {
      validate_url(url, &ingress_resource.config.allowed_networks)?
    }
    DomainTarget::Deployment { id, port, scheme } => {
      if *port == 0 {
        bail!("InvalidConfig: backend port is zero")
      }
      let deployment = crate::domain::authorized::<Deployment>(
        id,
        user,
        attach(SpecificPermission::AttachDomain),
      )
      .await?;
      let backend = if live {
        super::target::deployment(
          &deployment,
          *port,
          scheme,
          &ingress_resource,
        )
        .await?
      } else {
        validate_topology(
          &deployment.config.server_id,
          &deployment.config.swarm_id,
          &ingress_resource,
        )?;
        if deployment.config.network
          != ingress_resource.config.docker_network
        {
          bail!(
            "UnsupportedCapability: target network differs from ingress"
          )
        }
        String::new()
      };
      *id = deployment.id;
      backend
    }
    DomainTarget::StackService {
      stack_id,
      service,
      port,
      scheme,
    } => {
      let stack = crate::domain::authorized::<Stack>(
        stack_id,
        user,
        attach(SpecificPermission::AttachDomain),
      )
      .await?;
      if *port == 0
        || service.is_empty()
        || !service
          .chars()
          .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
      {
        bail!("InvalidConfig: invalid stack service")
      }
      let backend = if live {
        super::target::stack_service(
          &stack,
          service,
          *port,
          scheme,
          &ingress_resource,
        )
        .await?
      } else {
        validate_topology(
          &stack.config.server_id,
          &stack.config.swarm_id,
          &ingress_resource,
        )?;
        String::new()
      };
      *stack_id = stack.id;
      backend
    }
  };
  Ok((ingress_resource, provider, backend))
}
fn validate_topology(
  server: &str,
  swarm: &str,
  ingress: &Ingress,
) -> anyhow::Result<()> {
  let IngressAdapterConfig::TraefikFileV1 { server: edge, .. } =
    &ingress.config.adapter
  else {
    bail!(
      "UnsupportedCapability: Tunnel container targets require an explicit approved URL"
    )
  };
  if !swarm.is_empty()
    || server != edge
    || ingress.config.docker_network.is_empty()
    || ["bridge", "host", "none"]
      .contains(&ingress.config.docker_network.as_str())
  {
    bail!(
      "UnsupportedCapability: target must share a declared custom Docker network and server"
    )
  }
  Ok(())
}
pub fn validate_url(
  value: &str,
  allowed: &[String],
) -> anyhow::Result<String> {
  let url =
    url::Url::parse(value).context("InvalidTarget: malformed URL")?;
  if !matches!(url.scheme(), "http" | "https")
    || !url.username().is_empty()
    || url.password().is_some()
    || url.fragment().is_some()
    || url.host_str().is_none()
  {
    bail!(
      "InvalidTarget: only HTTP(S) URLs without credentials or fragments are allowed"
    )
  }
  let host = url
    .host_str()
    .unwrap()
    .trim_matches(['[', ']'])
    .trim_end_matches('.')
    .to_ascii_lowercase();
  if let Ok(address) = host.parse::<std::net::IpAddr>() {
    let forbidden = match address {
      std::net::IpAddr::V4(ip) => {
        ip.is_link_local() || ip.is_unspecified() || ip.is_loopback()
      }
      std::net::IpAddr::V6(ip) => {
        ip.is_unicast_link_local()
          || ip.is_unspecified()
          || ip.is_loopback()
          || ip.to_ipv4_mapped().is_some_and(|v| {
            v.is_link_local() || v.is_loopback() || v.is_unspecified()
          })
      }
    };
    if forbidden {
      bail!(
        "Forbidden: loopback, link-local and metadata targets are prohibited"
      )
    }
  }
  if host == "localhost"
    || host.ends_with(".localhost")
    || host == "localhost.localdomain"
    || host == "ip6-localhost"
  {
    bail!("Forbidden: localhost names are prohibited")
  }
  if host == "metadata.google.internal"
    || host == "169.254.169.254"
    || host == "100.100.100.200"
    || host == "fd00:ec2::254"
  {
    bail!("Forbidden: metadata target")
  }
  // Exact origins are an administrator trust boundary (DNS names remain mutable). A
  // configured origin grants this specific host and port, not arbitrary ports.
  let origin = url.origin().ascii_serialization();
  if !allowed.iter().any(|entry| entry == &origin) {
    bail!(
      "Forbidden: URL origin must be explicitly approved in ingress allowed_networks"
    )
  }
  Ok(url.into())
}
#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn normalize_and_reject() {
    assert_eq!(hostname("EXAMPLE.com.").unwrap(), "example.com");
    assert_eq!(
      hostname("bücher.example").unwrap(),
      "xn--bcher-kva.example"
    );
    for bad in [
      "*.example.com",
      "https://example.com",
      "a..com",
      "a.com..",
      " a.com",
      "a.com:80",
      "-a.com",
      "127.0.0.1",
    ] {
      assert!(hostname(bad).is_err(), "{bad}");
    }
  }
  #[test]
  fn suffix_boundary() {
    assert!(in_scope("a.example.com", &["example.com".into()]));
    assert!(!in_scope("badexample.com", &["example.com".into()]));
  }
  #[test]
  fn special_use_names_cannot_be_allowlisted() {
    for url in [
      "http://localhost",
      "http://app.localhost",
      "http://LOCALHOST.",
      "http://metadata.google.internal.",
      "http://METADATA.GOOGLE.INTERNAL",
      "http://[::ffff:169.254.169.254]",
      "http://[fd00:ec2::254]",
    ] {
      let origin =
        url::Url::parse(url).unwrap().origin().ascii_serialization();
      assert!(validate_url(url, &[origin]).is_err(), "{url}");
    }
  }
  #[test]
  fn url_scope() {
    let allow = vec!["http://app:8080".into()];
    assert!(validate_url("http://app:8080", &allow).is_ok());
    assert!(validate_url("http://app:8081", &allow).is_err());
    assert!(
      validate_url("http://user:pass@app:8080", &allow).is_err()
    );
    assert!(
      validate_url(
        "http://169.254.169.254",
        &["http://169.254.169.254".into()]
      )
      .is_err()
    );
  }
}

pub async fn normalize_provider(
  config: &mut DnsProviderConfig,
) -> anyhow::Result<()> {
  provider(config)
}
pub async fn normalize_ingress(
  config: &mut IngressConfig,
) -> anyhow::Result<()> {
  ingress(config)?;
  if let IngressAdapterConfig::TraefikFileV1 { server, .. } =
    &mut config.adapter
  {
    *server = crate::resource::get::<
      komodo_client::entities::server::Server,
    >(server)
    .await?
    .id;
  }
  Ok(())
}

/// Cleanup must remain possible when the application is stopped or missing from
/// Docker. It requires connection attachment rights but never exposes a target.
pub async fn resolve_for_operation(
  config: &mut DomainConfig,
  user: &User,
  operation: &DomainPlanOperation,
) -> anyhow::Result<(Ingress, Option<DnsProvider>, String)> {
  if !matches!(
    operation,
    DomainPlanOperation::Remove
      | DomainPlanOperation::Abandon
      | DomainPlanOperation::FinalizeMigration
  ) {
    return resolve(config, user).await;
  }
  enabled()?;
  let ingress = crate::domain::authorized::<Ingress>(
    &config.ingress,
    user,
    attach(SpecificPermission::Attach),
  )
  .await?;
  config.ingress = ingress.id.clone();
  let provider =
    if let DomainDns::Managed { provider, .. } = &mut config.dns {
      let resource = crate::domain::authorized::<DnsProvider>(
        provider,
        user,
        attach(SpecificPermission::Attach),
      )
      .await?;
      *provider = resource.id.clone();
      Some(resource)
    } else {
      None
    };
  Ok((ingress, provider, String::new()))
}
