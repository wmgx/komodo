//! Pure plan computation: desired vs observed vs owned -> `Vec<Change>`.
//! No I/O here, so it is cheap to test.

use komodo_client::entities::domain::{
  Change, ChangeAction, ChangeReason, DomainLayer, DomainReason,
  DomainStatus, ObjectRef, OwnedObject,
};
use serde_json::{Value, json};

use super::{
  error::code,
  provider::{RemoteRecord, RemoteRule, digest},
};

pub struct DnsInput<'a> {
  pub account: &'a str,
  /// Canonical desired record, see `provider::dns_canonical`
  pub desired: Value,
  /// Records at the hostname, with whether they carry our ownership marker.
  pub records: &'a [(RemoteRecord, bool)],
}

pub struct IngressInput<'a> {
  pub account: &'a str,
  /// `IngressTarget.id`
  pub scope: &'a str,
  pub service: &'a str,
  pub rule: Option<&'a RemoteRule>,
}

pub struct PlanInput<'a> {
  pub hostname: &'a str,
  pub owned: &'a [OwnedObject],
  pub dns: Option<DnsInput<'a>>,
  pub ingress: Option<IngressInput<'a>>,
}

fn dns_ref(key: &str, remote_id: Option<&str>) -> ObjectRef {
  ObjectRef {
    kind: "dns_record".to_string(),
    key: key.to_string(),
    remote_id: remote_id.map(str::to_string),
  }
}

fn rule_ref(key: &str) -> ObjectRef {
  ObjectRef {
    kind: "tunnel_rule".to_string(),
    key: key.to_string(),
    remote_id: None,
  }
}

fn rule_json(hostname: &str, service: &str) -> Value {
  json!({ "hostname": hostname, "service": service })
}

/// Changes in execution order: deletes (DNS before ingress),
/// then upserts (ingress before DNS).
pub fn compute_changes(input: &PlanInput) -> Vec<Change> {
  let mut changes = Vec::new();
  let hostname = input.hostname;

  // Owned objects which are no longer declared.
  for layer in [DomainLayer::Dns, DomainLayer::Ingress] {
    for owned in input.owned.iter().filter(|o| o.layer == layer) {
      let declared = match layer {
        DomainLayer::Dns => input.dns.as_ref().is_some_and(|d| {
          owned.key == hostname && owned.account == d.account
        }),
        DomainLayer::Ingress => {
          input.ingress.as_ref().is_some_and(|i| {
            owned.key == hostname
              && owned.account == i.account
              && owned.scope == i.scope
          })
        }
      };
      if declared {
        continue;
      }
      changes.push(Change {
        layer,
        object: match layer {
          DomainLayer::Dns => {
            dns_ref(&owned.key, Some(&owned.remote_id))
          }
          DomainLayer::Ingress => rule_ref(&owned.key),
        },
        action: ChangeAction::Delete,
        before: None,
        after: None,
        reason: Some(if owned.key != hostname {
          ChangeReason::HostnameChanged
        } else {
          ChangeReason::OrphanOwned
        }),
      });
    }
  }

  if let Some(ingress) = &input.ingress {
    changes.push(ingress_change(hostname, ingress, input.owned));
  }
  if let Some(dns) = &input.dns {
    changes.push(dns_change(hostname, dns, input.owned));
  }
  changes
}

fn ingress_change(
  hostname: &str,
  ingress: &IngressInput,
  owned: &[OwnedObject],
) -> Change {
  let owned = owned.iter().find(|o| {
    o.layer == DomainLayer::Ingress
      && o.key == hostname
      && o.account == ingress.account
      && o.scope == ingress.scope
  });
  let after = Some(rule_json(hostname, ingress.service));
  let (action, before, reason) = match ingress.rule {
    None => (ChangeAction::Create, None, Some(ChangeReason::Missing)),
    Some(rule) => {
      let before = Some(rule_json(hostname, &rule.service));
      match owned {
        None => (
          ChangeAction::Conflict,
          before,
          Some(ChangeReason::NotOwned),
        ),
        Some(_) if rule.service == ingress.service => {
          (ChangeAction::Noop, before, None)
        }
        Some(owned) => (
          ChangeAction::Update,
          before,
          Some(if digest(&rule.rule) != owned.applied_digest {
            ChangeReason::ManualEdit
          } else {
            ChangeReason::ContentDiffers
          }),
        ),
      }
    }
  };
  Change {
    layer: DomainLayer::Ingress,
    object: rule_ref(hostname),
    action,
    before,
    after,
    reason,
  }
}

fn dns_change(
  hostname: &str,
  dns: &DnsInput,
  owned: &[OwnedObject],
) -> Change {
  let owned = owned.iter().find(|o| {
    o.layer == DomainLayer::Dns
      && o.key == hostname
      && o.account == dns.account
  });
  // Ours: recorded id still exists, or it carries our marker.
  let ours = dns.records.iter().find(|(record, marked)| {
    *marked || owned.is_some_and(|o| o.remote_id == record.remote_id)
  });
  let after = Some(dns.desired.clone());
  let (action, remote_id, before, reason) = match ours {
    Some((record, _)) => {
      let remote_id = Some(record.remote_id.clone());
      let before = Some(record.canonical.clone());
      if record.canonical == dns.desired {
        (ChangeAction::Noop, remote_id, before, None)
      } else {
        let manual = owned.is_some_and(|o| {
          digest(&record.canonical) != o.applied_digest
        });
        (
          ChangeAction::Update,
          remote_id,
          before,
          Some(if manual {
            ChangeReason::ManualEdit
          } else {
            ChangeReason::ContentDiffers
          }),
        )
      }
    }
    None => match dns.records.first() {
      Some((record, _)) => (
        ChangeAction::Conflict,
        Some(record.remote_id.clone()),
        Some(record.canonical.clone()),
        Some(ChangeReason::NotOwned),
      ),
      None => (
        ChangeAction::Create,
        None,
        None,
        Some(ChangeReason::Missing),
      ),
    },
  };
  Change {
    layer: DomainLayer::Dns,
    object: dns_ref(hostname, remote_id.as_deref()),
    action,
    before,
    after,
    reason,
  }
}

/// Digest of the whole change list. Passed back to apply as `plan_id`.
pub fn plan_id(changes: &[Change]) -> String {
  digest(&serde_json::to_value(changes).unwrap_or(Value::Null))
}

/// Status implied by a plan.
pub fn status_of(
  changes: &[Change],
  errors: &[DomainReason],
) -> DomainStatus {
  if let Some(reason) = errors.first() {
    return DomainStatus::Error {
      reason: reason.clone(),
    };
  }
  let reasons: Vec<DomainReason> = changes
    .iter()
    .filter(|c| c.action != ChangeAction::Noop)
    .map(|c| {
      let (code, message) = match (c.layer, c.action) {
        (_, ChangeAction::Conflict) => (
          code::FOREIGN_CONFLICT,
          "A remote object exists which Komodo does not own",
        ),
        (DomainLayer::Dns, ChangeAction::Create) => {
          ("RecordMissing", "DNS record is missing")
        }
        (DomainLayer::Dns, _) => {
          ("RecordChanged", "DNS record differs from the config")
        }
        (DomainLayer::Ingress, ChangeAction::Create) => {
          ("RuleMissing", "Tunnel rule is missing")
        }
        (DomainLayer::Ingress, _) => {
          ("RuleChanged", "Tunnel rule differs from the config")
        }
      };
      DomainReason {
        code: code.to_string(),
        layer: Some(c.layer),
        object_key: Some(c.object.key.clone()),
        message: message.to_string(),
        retryable: false,
      }
    })
    .collect();
  if reasons.is_empty() {
    DomainStatus::Ready
  } else {
    DomainStatus::Drifted { reasons }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::domain::provider::dns_canonical;

  fn record(id: &str, content: &str) -> RemoteRecord {
    RemoteRecord {
      remote_id: id.into(),
      scope: "zone".into(),
      canonical: dns_canonical("CNAME", content, 1, true),
      marker: None,
    }
  }

  fn owned_dns(record: &RemoteRecord) -> OwnedObject {
    OwnedObject {
      layer: DomainLayer::Dns,
      account: "cf".into(),
      key: "app.example.com".into(),
      remote_id: record.remote_id.clone(),
      scope: "zone".into(),
      applied_digest: digest(&record.canonical),
    }
  }

  fn dns_input<'a>(
    records: &'a [(RemoteRecord, bool)],
  ) -> DnsInput<'a> {
    DnsInput {
      account: "cf",
      desired: dns_canonical("CNAME", "t.cfargotunnel.com", 0, true),
      records,
    }
  }

  fn plan<'a>(
    owned: &'a [OwnedObject],
    dns: DnsInput<'a>,
  ) -> Vec<Change> {
    compute_changes(&PlanInput {
      hostname: "app.example.com",
      owned,
      dns: Some(dns),
      ingress: None,
    })
  }

  #[test]
  fn create_then_noop_then_manual_edit_drift() {
    // Nothing remote, nothing owned -> Create, drifted.
    let changes = plan(&[], dns_input(&[]));
    assert_eq!(changes[0].action, ChangeAction::Create);
    assert!(matches!(
      status_of(&changes, &[]),
      DomainStatus::Drifted { .. }
    ));

    // Owned and identical -> Noop, ready.
    let same = record("r1", "t.cfargotunnel.com");
    let owned = [owned_dns(&same)];
    let records = [(same, true)];
    let changes = plan(&owned, dns_input(&records));
    assert_eq!(changes[0].action, ChangeAction::Noop);
    assert_eq!(status_of(&changes, &[]), DomainStatus::Ready);

    // Someone edited the owned record -> Update / ManualEdit.
    let original = record("r1", "t.cfargotunnel.com");
    let owned = [owned_dns(&original)];
    let records = [(record("r1", "evil.example.net"), true)];
    let changes = plan(&owned, dns_input(&records));
    assert_eq!(changes[0].action, ChangeAction::Update);
    assert_eq!(changes[0].reason, Some(ChangeReason::ManualEdit));
  }

  #[test]
  fn foreign_record_is_a_conflict() {
    let records = [(record("x", "other.example.net"), false)];
    let changes = plan(&[], dns_input(&records));
    assert_eq!(changes[0].action, ChangeAction::Conflict);
    assert_eq!(changes[0].reason, Some(ChangeReason::NotOwned));
  }
}
