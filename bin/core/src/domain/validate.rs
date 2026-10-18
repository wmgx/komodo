//! Local (no network) validation of Domain config.

use komodo_client::entities::{
  ResourceTarget, domain::PartialDomainConfig,
};

use super::error::DomainError;

/// Lowercase, strip the trailing dot, and check it is an exact
/// (non wildcard) ASCII hostname with at least two labels.
pub fn normalize_hostname(
  hostname: &str,
) -> Result<String, DomainError> {
  let hostname =
    hostname.trim().trim_end_matches('.').to_ascii_lowercase();
  let labels: Vec<&str> = hostname.split('.').collect();
  let label_ok = |label: &&str| {
    !label.is_empty()
      && label.len() <= 63
      && !label.starts_with('-')
      && !label.ends_with('-')
      && label
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
  };
  if hostname.len() > 253
    || labels.len() < 2
    || !labels.iter().all(label_ok)
  {
    return Err(DomainError::invalid(format!(
      "'{hostname}' is not a valid exact hostname (eg. app.example.com)"
    )));
  }
  Ok(hostname)
}

/// Normalizes `hostname` in place and checks the set fields.
/// Cross-field and account checks happen when planning.
pub fn validate_partial(
  config: &mut PartialDomainConfig,
) -> Result<(), DomainError> {
  if let Some(hostname) = &mut config.hostname {
    *hostname = normalize_hostname(hostname)?;
  }
  if let Some(dns) = &config.dns
    && dns.ttl != 0
    && !(60..=86400).contains(&dns.ttl)
  {
    return Err(DomainError::invalid(
      "dns.ttl must be 0 (auto) or between 60 and 86400",
    ));
  }
  if let Some(ingress) = &config.ingress
    && let Some(target) = &ingress.target
    && !matches!(
      target,
      ResourceTarget::Deployment(_) | ResourceTarget::Stack(_)
    )
  {
    return Err(DomainError::invalid(
      "ingress.target must be a Deployment or Stack",
    ));
  }
  Ok(())
}
