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
  let bad = |why: String| {
    DomainError::invalid(format!(
      "Invalid hostname '{hostname}': {why}"
    ))
  };
  if hostname.len() > 253 {
    return Err(bad("it is longer than 253 characters".into()));
  }
  let labels: Vec<&str> = hostname.split('.').collect();
  if labels.len() < 2 {
    return Err(bad(
      "it needs a domain suffix, eg. app.example.com".into(),
    ));
  }
  for label in labels {
    if label.is_empty() {
      return Err(bad(
        "it has an empty part (two dots in a row)".into(),
      ));
    }
    if label.len() > 63 {
      return Err(bad(format!(
        "'{label}' is longer than 63 characters"
      )));
    }
    if label.contains('_') {
      return Err(bad(format!(
        "'{label}' contains an underscore, which DNS hostnames do not allow. Use a hyphen instead, eg. '{}'",
        label.replace('_', "-")
      )));
    }
    if label.starts_with('-') || label.ends_with('-') {
      return Err(bad(format!(
        "'{label}' cannot start or end with a hyphen"
      )));
    }
    if let Some(c) = label
      .chars()
      .find(|c| !(c.is_ascii_alphanumeric() || *c == '-'))
    {
      return Err(bad(format!(
        "'{label}' contains '{c}'; only letters, digits and hyphens are allowed"
      )));
    }
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
  if let Some(target) = &config.target
    && !matches!(
      target,
      ResourceTarget::Deployment(_) | ResourceTarget::Stack(_)
    )
  {
    return Err(DomainError::invalid(
      "target must be a Deployment or Stack",
    ));
  }
  Ok(())
}
