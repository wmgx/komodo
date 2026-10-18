//! Merge one hostname rule into a Cloudflare tunnel ingress configuration.
//!
//! Only the rule for `hostname` is touched. Unknown fields, every other rule,
//! and the final catch-all are preserved byte for byte.

use serde_json::{Value, json};

use crate::domain::error::{ProviderError, code};

fn malformed() -> ProviderError {
  ProviderError::new(
    code::PROVIDER_RESPONSE,
    "Tunnel configuration has no ingress rules",
  )
}

/// Add / update (or with `remove`, delete) the rule for `hostname`.
/// Ownership is decided by the caller, not here.
pub fn merge(
  config: &Value,
  hostname: &str,
  service: &str,
  remove: bool,
) -> Result<Value, ProviderError> {
  if !remove && service.starts_with("https://") {
    let per_rule = config["ingress"]
      .as_array()
      .and_then(|rules| {
        rules.iter().find(|rule| rule["hostname"] == hostname)
      })
      .and_then(|rule| rule.get("originRequest"))
      .and_then(|origin| origin.get("noTLSVerify"));
    let insecure = per_rule
      .or_else(|| {
        config
          .get("originRequest")
          .and_then(|origin| origin.get("noTLSVerify"))
      })
      .and_then(Value::as_bool)
      .unwrap_or(false);
    if insecure {
      return Err(ProviderError::invalid(
        "HTTPS origin certificate verification must remain enabled (noTLSVerify)",
      ));
    }
  }

  let mut next = config.clone();
  let rules = next["ingress"].as_array_mut().ok_or_else(malformed)?;

  let has_catch_all = rules.last().is_some_and(|last| {
    last.get("hostname").is_none() && last.get("path").is_none()
  });
  if !has_catch_all {
    return Err(ProviderError::invalid(
      "Tunnel configuration must end with a catch-all rule",
    ));
  }

  // An earlier wildcard could capture this host.
  // Never silently shadow a manually maintained rule.
  if !remove
    && rules.iter().any(|rule| {
      rule["hostname"].as_str().is_some_and(|h| {
        h == "*"
          || (h.starts_with("*.") && hostname.ends_with(&h[1..]))
          || (h.contains('*') && !h.starts_with("*."))
      })
    })
  {
    return Err(ProviderError::new(
      code::FOREIGN_CONFLICT,
      "A wildcard rule in the tunnel already matches this hostname",
    ));
  }

  let matching: Vec<usize> = rules
    .iter()
    .enumerate()
    .filter(|(_, rule)| rule["hostname"] == hostname)
    .map(|(i, _)| i)
    .collect();
  if matching.len() > 1 {
    return Err(ProviderError::new(
      code::FOREIGN_CONFLICT,
      "Multiple rules already exist for this hostname",
    ));
  }

  match (matching.first(), remove) {
    (Some(&index), true) => {
      rules.remove(index);
    }
    (Some(&index), false) => {
      rules[index]["service"] = json!(service);
    }
    (None, true) => {}
    (None, false) => {
      // Before the catch-all, which is verified to exist above.
      let at = rules.len() - 1;
      rules.insert(
        at,
        json!({ "hostname": hostname, "service": service }),
      );
    }
  }
  Ok(next)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn preserves_unknown_fields_and_catch_all() {
    let cfg = json!({
      "originRequest": { "connectTimeout": 10 },
      "future": 123,
      "ingress": [
        { "hostname": "manual.test", "service": "http://manual:80", "future": true },
        { "service": "http_status:404" }
      ]
    });
    let next =
      merge(&cfg, "new.test", "http://app:80", false).unwrap();
    assert_eq!(next["future"], 123);
    assert_eq!(next["ingress"][0], cfg["ingress"][0]);
    assert_eq!(next["ingress"][1]["hostname"], "new.test");
    assert_eq!(next["ingress"][2], cfg["ingress"][1]);
    // Idempotent, and removal restores the original exactly.
    assert_eq!(
      merge(&next, "new.test", "http://app:80", false).unwrap(),
      next
    );
    assert_eq!(merge(&next, "new.test", "", true).unwrap(), cfg);
  }

  #[test]
  fn rejects_missing_catch_all() {
    assert!(
      merge(&json!({ "ingress": [] }), "a.test", "http://a", false)
        .is_err()
    );
  }
}
