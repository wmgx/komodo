//! "Give this service a hostname" in one step: resolve the target,
//! derive the config ([super::auto]), preview the plan, create the Domain
//! and apply it.

use database::mungos::mongodb::bson::doc;
use komodo_client::entities::{
  Operation, ResourceTarget,
  domain::{
    ChangeAction, Domain, DomainConfig, DomainMode,
    ServiceDomainResult,
  },
  user::User,
};

use crate::{
  helpers::update::{add_update, make_update},
  resource,
  state::{all_resources_cache, db_client},
};

use super::{
  accounts,
  apply::apply,
  auto::{AccountCtx, DeriveInput, ServerInfo, derive, host_of},
  error::DomainError,
  lock::lock_accounts,
  prepare::{account_names_of, find_by_id_or_name, make_plan},
  provider::{ServerRef, factory},
  validate::normalize_hostname,
};

pub struct ResolvedTarget {
  pub target: ResourceTarget,
  pub service: String,
  /// Published port found on the target itself (Deployment config).
  pub detected_port: u16,
  pub server: Option<ServerInfo>,
}

/// `host:container` lines of a Deployment; the host port if exactly one.
fn single_host_port(ports: &str) -> u16 {
  let found: Vec<u16> = ports
    .lines()
    .map(|l| l.split('#').next().unwrap_or("").trim())
    .filter(|l| !l.is_empty())
    .filter_map(|l| {
      let l = l.split('/').next().unwrap_or(l);
      let parts: Vec<&str> = l.split(':').collect();
      parts
        .len()
        .checked_sub(2)
        .and_then(|i| parts[i].parse().ok())
    })
    .collect();
  match found.as_slice() {
    [one] => *one,
    _ => 0,
  }
}

pub fn resolve_target(
  target: &ResourceTarget,
  service: &str,
) -> Result<ResolvedTarget, DomainError> {
  let all = all_resources_cache().load();
  let missing = |kind: &str, id: &str| {
    DomainError::invalid(format!("{kind} '{id}' 不存在"))
  };
  let (target, service, server_id, detected_port) = match target {
    ResourceTarget::Deployment(id) => {
      let d = find_by_id_or_name(&all.deployments, id, |d| &d.name)
        .ok_or_else(|| missing("Deployment", id))?;
      (
        ResourceTarget::Deployment(d.id.clone()),
        String::new(),
        d.config.server_id.clone(),
        single_host_port(&d.config.ports),
      )
    }
    ResourceTarget::Stack(id) => {
      let s = find_by_id_or_name(&all.stacks, id, |s| &s.name)
        .ok_or_else(|| missing("Stack", id))?;
      if service.is_empty() {
        return Err(DomainError::invalid(
          "Stack 需要指定 service（compose 里的服务名）",
        ));
      }
      (
        ResourceTarget::Stack(s.id.clone()),
        service.to_string(),
        s.config.server_id.clone(),
        0,
      )
    }
    _ => {
      return Err(DomainError::invalid(
        "target 必须是 Deployment 或 Stack",
      ));
    }
  };
  let server =
    find_by_id_or_name(&all.servers, &server_id, |s| &s.name).map(
      |server| {
        let address = if server.config.external_address.is_empty() {
          &server.config.address
        } else {
          &server.config.external_address
        };
        ServerInfo {
          server: ServerRef {
            id: server.id.clone(),
            name: server.name.clone(),
          },
          host: host_of(address),
        }
      },
    );
  Ok(ResolvedTarget {
    target,
    service,
    detected_port,
    server,
  })
}

/// All accounts with providers and their usable zones.
/// Zone discovery (network, read only) only happens when it is needed
/// to choose between several DNS accounts.
async fn gather_accounts() -> Result<Vec<AccountCtx>, DomainError> {
  let all = accounts::list_all().await?;
  let mut ctxs = Vec::new();
  for account in all {
    let Ok(set) = account.providers() else {
      continue;
    };
    let mut zones: Vec<String> = account.params["zones"]
      .as_array()
      .map(|a| {
        a.iter()
          .filter_map(|z| z.as_str().map(str::to_ascii_lowercase))
          .collect()
      })
      .unwrap_or_default();
    ctxs.push((account, set, std::mem::take(&mut zones)));
  }
  let need_discovery =
    ctxs.iter().filter(|(_, s, _)| s.dns.is_some()).count() > 1;
  let mut out = Vec::new();
  for (account, set, mut zones) in ctxs {
    if need_discovery
      && zones.is_empty()
      && set.dns.is_some()
      && let Some(discover) =
        factory(&account.kind).and_then(|f| f.discover)
    {
      match discover(account.token.clone()).await {
        Ok(d) => zones = d.zones,
        Err(e) => e.log("discover zones for AddServiceDomain"),
      }
    }
    out.push(AccountCtx {
      name: account.name,
      dns: set.dns,
      ingress: set.ingress,
      zones,
    });
  }
  Ok(out)
}

async fn unique_name(hostname: &str) -> Result<String, DomainError> {
  let base = hostname.replace('.', "-");
  for n in 1..100 {
    let name = if n == 1 {
      base.clone()
    } else {
      format!("{base}-{n}")
    };
    let exists = db_client()
      .domains
      .find_one(doc! { "name": &name })
      .await
      .map_err(DomainError::internal)?
      .is_some();
    if !exists {
      return Ok(name);
    }
  }
  Err(DomainError::internal("no free Domain name"))
}

pub struct AddRequest {
  pub target: ResourceTarget,
  pub service: String,
  pub hostname: String,
  pub port: u16,
  pub mode: DomainMode,
  pub adopt_conflicts: bool,
}

pub async fn add_service_domain(
  req: AddRequest,
  user: &User,
) -> Result<ServiceDomainResult, DomainError> {
  let hostname = normalize_hostname(&req.hostname)?;
  let resolved = resolve_target(&req.target, &req.service)?;

  let exists = db_client()
    .domains
    .find_one(doc! { "config.hostname": &hostname })
    .await
    .map_err(DomainError::internal)?;
  if let Some(existing) = exists {
    return Err(DomainError::invalid(format!(
      "主机名 '{hostname}' 已被 Domain '{}' 使用，请直接修改或删除它。",
      existing.name
    )));
  }

  let accounts = gather_accounts().await?;
  let port = if req.port == 0 {
    resolved.detected_port
  } else {
    req.port
  };
  let derived = derive(&DeriveInput {
    hostname: &hostname,
    target: resolved.target,
    service: resolved.service,
    port,
    mode: req.mode,
    server: resolved.server,
    accounts: &accounts,
  })?;
  let mut result = ServiceDomainResult {
    mode: Some(derived.mode),
    warnings: derived.warnings,
    ..Default::default()
  };

  // Preview on a throwaway Domain: reads the remote side only.
  let preview = Domain {
    config: derived.config.clone(),
    ..Default::default()
  };
  let planned = {
    let _guards = lock_accounts(
      account_names_of(&preview).iter().map(String::as_str),
    )
    .await;
    make_plan(&preview).await
  };
  if let Some(reason) = planned.plan.errors.into_iter().next() {
    result.error = Some(reason.message.clone());
    result.reason = Some(reason);
    return Ok(result);
  }
  let conflicts: Vec<_> = planned
    .plan
    .changes
    .into_iter()
    .filter(|c| c.action == ChangeAction::Conflict)
    .collect();
  if !conflicts.is_empty() && !req.adopt_conflicts {
    result.conflicts = conflicts;
    return Ok(result);
  }

  let name = unique_name(&hostname).await?;
  let created = resource::create::<Domain>(
    &name,
    DomainConfig::into(derived.config),
    None,
    user,
  )
  .await
  .map_err(|e| DomainError::internal(format!("{e:?}")))?;

  let mut update = make_update(
    ResourceTarget::Domain(created.id.clone()),
    Operation::ApplyDomain,
    user,
  );
  match apply(&created, None, req.adopt_conflicts).await {
    Ok(outcome) => {
      update.push_simple_log(
        "Plan",
        serde_json::to_string_pretty(&outcome.plan)
          .unwrap_or_default(),
      );
      if let Some(reason) = outcome.error {
        update.push_error_log(
          "Apply failed",
          serde_json::to_string_pretty(&reason).unwrap_or_default(),
        );
        result.error = Some(reason.message.clone());
        result.reason = Some(reason);
      }
    }
    Err(e) => {
      e.log("AddServiceDomain apply");
      update.push_error_log(
        "Apply failed",
        format!("{}: {}", e.code, e.message),
      );
      result.error = Some(e.message);
    }
  }
  update.finalize();
  if let Err(e) = add_update(update).await {
    error!("Failed to add update for AddServiceDomain | {e:#}");
  }
  result.domain = resource::get::<Domain>(&created.id).await.ok();
  Ok(result)
}
