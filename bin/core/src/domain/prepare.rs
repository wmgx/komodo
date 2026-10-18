//! Turns a stored `Domain` into providers + a fully resolved desired state,
//! then observes the remote side to produce a [DomainPlan].

use komodo_client::entities::{
  ResourceTarget,
  domain::{
    Domain, DomainLayer, DomainPlan, DomainReason, OwnedObject,
  },
  komodo_timestamp,
};

use crate::{resource, state::all_resources_cache};

use super::{
  accounts,
  error::{DomainError, code},
  lock::lock_accounts,
  plan::{
    DnsInput, IngressInput, PlanInput, compute_changes, plan_id,
    status_of,
  },
  provider::{
    DnsProvider, DnsRecordSpec, IngressProvider, IngressTarget,
    Owner, RemoteRecord, RemoteRule, ServerRef,
  },
  store,
  validate::normalize_hostname,
};

/// A failure tied to a layer.
pub struct LayerError {
  pub layer: Option<DomainLayer>,
  pub error: DomainError,
}

impl LayerError {
  fn dns(error: DomainError) -> Self {
    Self {
      layer: Some(DomainLayer::Dns),
      error,
    }
  }
  fn ingress(error: DomainError) -> Self {
    Self {
      layer: Some(DomainLayer::Ingress),
      error,
    }
  }
  pub fn to_reason(&self, key: &str) -> DomainReason {
    self.error.to_reason(self.layer, Some(key))
  }
}

pub struct PreparedDns {
  pub account: String,
  pub provider: Box<dyn DnsProvider>,
  pub spec: DnsRecordSpec,
}

pub struct PreparedIngress {
  pub account: String,
  pub provider: Box<dyn IngressProvider>,
  pub target: IngressTarget,
  pub service: String,
}

/// Everything needed to plan / apply, without any remote reads yet.
pub struct Prepared {
  pub hostname: String,
  pub owner: Owner,
  pub owned: Vec<OwnedObject>,
  pub dns: Option<PreparedDns>,
  pub ingress: Option<PreparedIngress>,
}

/// Accounts named by the config and by leftovers, for locking before `prepare`.
pub fn account_names_of(domain: &Domain) -> Vec<String> {
  let mut names: Vec<String> =
    [&domain.config.dns.account, &domain.config.ingress.account]
      .into_iter()
      .filter(|a| !a.is_empty())
      .cloned()
      .chain(domain.info.owned.iter().map(|o| o.account.clone()))
      .collect();
  names.sort();
  names.dedup();
  names
}

fn find_by_id_or_name<'a, T>(
  map: &'a std::collections::HashMap<String, T>,
  id_or_name: &str,
  name_of: impl Fn(&T) -> &str,
) -> Option<&'a T> {
  map
    .get(id_or_name)
    .or_else(|| map.values().find(|v| name_of(v) == id_or_name))
}

/// The Server a Deployment / Stack runs on.
fn server_of(
  target: &ResourceTarget,
) -> Result<Option<ServerRef>, DomainError> {
  let all = all_resources_cache().load();
  let not_found = |kind: &str, id: &str| {
    DomainError::invalid(format!(
      "ingress.target: {kind} '{id}' does not exist"
    ))
  };
  let server_id = match target {
    ResourceTarget::Deployment(id) => {
      find_by_id_or_name(&all.deployments, id, |d| &d.name)
        .ok_or_else(|| not_found("Deployment", id))?
        .config
        .server_id
        .clone()
    }
    ResourceTarget::Stack(id) => {
      find_by_id_or_name(&all.stacks, id, |s| &s.name)
        .ok_or_else(|| not_found("Stack", id))?
        .config
        .server_id
        .clone()
    }
    _ => {
      return Err(DomainError::invalid(
        "ingress.target must be a Deployment or Stack",
      ));
    }
  };
  if server_id.is_empty() {
    // Swarm based, nothing to bind a tunnel to.
    return Ok(None);
  }
  Ok(
    find_by_id_or_name(&all.servers, &server_id, |s| &s.name).map(
      |server| ServerRef {
        id: server.id.clone(),
        name: server.name.clone(),
      },
    ),
  )
}

/// Resolve accounts, providers, tunnel and the desired record.
/// No remote reads.
pub async fn prepare(
  domain: &Domain,
) -> Result<Prepared, LayerError> {
  let config = &domain.config;
  let hostname =
    normalize_hostname(&config.hostname).map_err(|e| LayerError {
      layer: None,
      error: e,
    })?;

  let mut ingress = None;
  if !config.ingress.account.is_empty() {
    let binding = &config.ingress;
    let account = accounts::require(&binding.account)
      .await
      .map_err(LayerError::ingress)?;
    let provider = account
      .providers()
      .and_then(|set| {
        set.ingress.ok_or_else(|| {
          DomainError::new(
            code::ACCOUNT_KIND_MISMATCH,
            format!(
              "Account '{}' cannot provide tunnel ingress",
              account.name
            ),
          )
        })
      })
      .map_err(LayerError::ingress)?;
    provider
      .validate(&binding.service)
      .map_err(LayerError::ingress)?;
    let server = match (&binding.target, binding.tunnel.is_empty()) {
      (Some(target), true) => {
        server_of(target).map_err(LayerError::ingress)?
      }
      _ => None,
    };
    let target = provider
      .select(Some(binding.tunnel.as_str()), server.as_ref())
      .map_err(LayerError::ingress)?;
    ingress = Some(PreparedIngress {
      account: account.name,
      provider,
      target,
      service: binding.service.clone(),
    });
  }

  let mut dns = None;
  if !config.dns.account.is_empty() {
    let binding = &config.dns;
    let account = accounts::require(&binding.account)
      .await
      .map_err(LayerError::dns)?;
    let provider = account
      .providers()
      .and_then(|set| {
        set.dns.ok_or_else(|| {
          DomainError::new(
            code::ACCOUNT_KIND_MISMATCH,
            format!("Account '{}' cannot manage DNS", account.name),
          )
        })
      })
      .map_err(LayerError::dns)?;
    // Empty content / unset proxied come from the tunnel.
    let dns_target =
      ingress.as_ref().and_then(|i| i.target.dns_target.as_ref());
    let (record_type, content, proxied) =
      if binding.content.is_empty() {
        let target = dns_target.ok_or_else(|| {
          LayerError::dns(DomainError::invalid(
            "dns.content is required when no tunnel is configured",
          ))
        })?;
        (
          target.record_type,
          target.content.clone(),
          binding.proxied.unwrap_or(target.proxied),
        )
      } else {
        (
          binding.record_type,
          binding.content.clone(),
          binding.proxied.unwrap_or(false),
        )
      };
    let spec = DnsRecordSpec {
      record_type,
      content,
      ttl: if binding.ttl == 0 { 1 } else { binding.ttl },
      proxied,
    };
    provider
      .validate(&hostname, &spec)
      .map_err(LayerError::dns)?;
    dns = Some(PreparedDns {
      account: account.name,
      provider,
      spec,
    });
  }

  Ok(Prepared {
    hostname,
    owner: Owner {
      id: domain.id.clone(),
    },
    owned: domain.info.owned.clone(),
    dns,
    ingress,
  })
}

/// What was read from the remote side.
pub struct Observed {
  pub records: Vec<(RemoteRecord, bool)>,
  pub rule: Option<RemoteRule>,
}

/// Read only the objects named by this domain. Other hostnames, records
/// and tunnel rules are never compared.
pub async fn observe(
  prepared: &Prepared,
) -> Result<Observed, LayerError> {
  let mut records = Vec::new();
  if let Some(dns) = &prepared.dns {
    records = dns
      .provider
      .list(&prepared.hostname)
      .await
      .map_err(LayerError::dns)?
      .into_iter()
      .map(|r| {
        let ours = dns.provider.is_owned(&r, &prepared.owner);
        (r, ours)
      })
      .collect();
  }
  let mut rule = None;
  if let Some(ingress) = &prepared.ingress {
    rule = ingress
      .provider
      .get(&ingress.target, &prepared.hostname)
      .await
      .map_err(LayerError::ingress)?;
  }
  Ok(Observed { records, rule })
}

pub fn plan_input<'a>(
  prepared: &'a Prepared,
  observed: &'a Observed,
) -> PlanInput<'a> {
  PlanInput {
    hostname: &prepared.hostname,
    owned: &prepared.owned,
    dns: prepared.dns.as_ref().map(|d| DnsInput {
      account: &d.account,
      desired: d.spec.canonical(),
      records: &observed.records,
    }),
    ingress: prepared.ingress.as_ref().map(|i| IngressInput {
      account: &i.account,
      scope: &i.target.id,
      service: &i.service,
      rule: observed.rule.as_ref(),
    }),
  }
}

/// A plan which only reports `error`.
pub fn error_plan(
  domain_id: &str,
  reason: DomainReason,
) -> DomainPlan {
  DomainPlan {
    domain: domain_id.to_string(),
    generated_at: komodo_timestamp(),
    plan_id: plan_id(&[]),
    changes: Vec::new(),
    errors: vec![reason],
  }
}

/// Everything a plan produces, kept together so apply can reuse it.
pub struct Planned {
  pub plan: DomainPlan,
  pub prepared: Option<Prepared>,
  pub observed: Option<Observed>,
}

/// prepare + observe + diff. Never fails: problems become `plan.errors`.
/// Caller must already hold the account locks.
pub async fn make_plan(domain: &Domain) -> Planned {
  let key = domain.config.hostname.as_str();
  let prepared = match prepare(domain).await {
    Ok(p) => p,
    Err(e) => {
      e.error.log(&format!("prepare domain {}", domain.name));
      return Planned {
        plan: error_plan(&domain.id, e.to_reason(key)),
        prepared: None,
        observed: None,
      };
    }
  };
  let observed = match observe(&prepared).await {
    Ok(o) => o,
    Err(e) => {
      e.error.log(&format!("observe domain {}", domain.name));
      return Planned {
        plan: error_plan(&domain.id, e.to_reason(&prepared.hostname)),
        prepared: Some(prepared),
        observed: None,
      };
    }
  };
  let changes = compute_changes(&plan_input(&prepared, &observed));
  Planned {
    plan: DomainPlan {
      domain: domain.id.clone(),
      generated_at: komodo_timestamp(),
      plan_id: plan_id(&changes),
      changes,
      errors: Vec::new(),
    },
    prepared: Some(prepared),
    observed: Some(observed),
  }
}

/// Plan under the account locks and cache the resulting status.
/// Only reads from providers.
pub async fn plan_domain(
  domain: &Domain,
) -> Result<DomainPlan, DomainError> {
  let _guards = lock_accounts(
    account_names_of(domain).iter().map(String::as_str),
  )
  .await;
  // Re-read under the lock: `owned` may have changed meanwhile.
  let domain = resource::get::<Domain>(&domain.id)
    .await
    .map_err(DomainError::internal)?;
  let planned = make_plan(&domain).await;
  let status = status_of(&planned.plan.changes, &planned.plan.errors);
  if let Err(e) = store::save_status(&domain.id, &status).await {
    e.log("save domain status");
  }
  Ok(planned.plan)
}
