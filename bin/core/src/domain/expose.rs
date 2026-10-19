//! "Make this service public" in one call: open the connection of the
//! service's Server when needed, then add `<prefix>.<zone>` for it.
//! Everything is composed from [tunnel] and [service]; this module only
//! chooses the missing inputs (name, domain, port) and reports progress.

use std::{
  collections::HashMap,
  sync::{LazyLock, Mutex},
  time::Duration,
};

use database::mungos::{find::find_collect, mongodb::bson::doc};
use komodo_client::entities::{
  ResourceTarget,
  domain::{Domain, DomainMode, ExposeServiceResult, ExposeStage},
  domain_account::ServerTunnelState,
  user::User,
};

use crate::{
  resource,
  state::{all_resources_cache, db_client},
};

use super::{
  accounts,
  error::DomainError,
  listing::{choose_port, service_ports},
  prepare::find_by_id_or_name,
  provider::factory,
  service::{AddRequest, add_service_domain, resolve_target},
  tunnel::{self, EnableRequest},
};

static PROGRESS: LazyLock<Mutex<HashMap<String, ExposeStage>>> =
  LazyLock::new(Default::default);

fn key(target: &ResourceTarget, service: &str) -> String {
  format!("{}/{service}", target.extract_variant_id().1)
}

/// Stage of a running `expose` for this service.
pub fn stage_of(
  target: &ResourceTarget,
  service: &str,
) -> Option<ExposeStage> {
  PROGRESS.lock().ok()?.get(&key(target, service)).copied()
}

/// Removes the progress entry when the call ends, however it ends.
struct Progress(String);

impl Progress {
  fn start(key: String) -> Result<Self, DomainError> {
    let mut map = PROGRESS
      .lock()
      .map_err(|_| DomainError::internal("progress lock poisoned"))?;
    if map.contains_key(&key) {
      return Err(DomainError::invalid(
        "This service is already being made public. Wait for it to finish.",
      ));
    }
    map.insert(key.clone(), ExposeStage::Connecting);
    Ok(Self(key))
  }

  fn set(&self, stage: ExposeStage) {
    if let Ok(mut map) = PROGRESS.lock() {
      map.insert(self.0.clone(), stage);
    }
  }
}

impl Drop for Progress {
  fn drop(&mut self) {
    if let Ok(mut map) = PROGRESS.lock() {
      map.remove(&self.0);
    }
  }
}

/// Lowercase letters, digits and `-`: anything else becomes `-`.
pub fn sanitize_label(name: &str) -> String {
  let label: String = name
    .trim()
    .chars()
    .map(|c| {
      let c = c.to_ascii_lowercase();
      if c.is_ascii_lowercase() || c.is_ascii_digit() {
        c
      } else {
        '-'
      }
    })
    .collect();
  label.trim_matches('-').chars().take(63).collect()
}

/// Domains the accounts can manage: the account's allow-list, else
/// whatever the token sees (read only).
async fn available_zones() -> Result<Vec<String>, DomainError> {
  let mut zones = Vec::<String>::new();
  for account in accounts::list_all().await? {
    let Ok(set) = account.providers() else {
      continue;
    };
    if set.dns.is_none() {
      continue;
    }
    let listed: Vec<String> = account.params["zones"]
      .as_array()
      .map(|a| {
        a.iter()
          .filter_map(|z| z.as_str().map(str::to_ascii_lowercase))
          .collect()
      })
      .unwrap_or_default();
    if !listed.is_empty() {
      zones.extend(listed);
    } else if let Some(discover) =
      factory(&account.kind).and_then(|f| f.discover)
    {
      match discover(account.token.clone()).await {
        Ok(found) => zones.extend(found.zones),
        Err(e) => e.log("discover zones for ExposeService"),
      }
    }
  }
  zones.sort();
  zones.dedup();
  Ok(zones)
}

async fn choose_zone(
  zone: Option<&str>,
) -> Result<String, DomainError> {
  if let Some(zone) = zone.map(str::trim).filter(|z| !z.is_empty()) {
    return Ok(zone.trim_matches('.').to_ascii_lowercase());
  }
  let mut zones = available_zones().await?;
  match zones.len() {
    0 => Err(DomainError::invalid(
      "No domain is available. Add a Cloudflare account with a domain in Settings -> Domains first.",
    )),
    1 => Ok(zones.remove(0)),
    _ => Err(DomainError::invalid(format!(
      "Several domains are available ({}). Choose one.",
      zones.join(", ")
    ))),
  }
}

pub struct ExposeRequest {
  pub target: ResourceTarget,
  pub service: String,
  pub prefix: Option<String>,
  pub zone: Option<String>,
  pub port: u16,
  pub adopt_conflicts: bool,
}

/// Give the connector a moment to come up, so the link works right away.
/// Never fails: the address is set up either way.
async fn wait_connected(server_id: &str) {
  for _ in 0..15 {
    match tunnel::status(server_id).await {
      Ok(t)
        if matches!(
          t.state,
          ServerTunnelState::Healthy | ServerTunnelState::Degraded
        ) =>
      {
        return;
      }
      Err(_) => return,
      Ok(_) => tokio::time::sleep(Duration::from_secs(2)).await,
    }
  }
}

pub async fn expose(
  req: ExposeRequest,
  user: &User,
) -> Result<ExposeServiceResult, DomainError> {
  let resolved = resolve_target(&req.target, &req.service)?;
  let Some(server) =
    resolved.server.as_ref().map(|s| s.server.clone())
  else {
    return Err(DomainError::invalid(
      "This service is not on a single server (Swarm is not supported yet).",
    ));
  };
  let progress =
    Progress::start(key(&resolved.target, &resolved.service))?;

  let name = match &resolved.target {
    ResourceTarget::Deployment(id) => {
      let all = all_resources_cache().load();
      find_by_id_or_name(&all.deployments, id, |d| &d.name)
        .map(|d| d.name.clone())
        .unwrap_or_default()
    }
    _ => resolved.service.clone(),
  };
  let prefix = sanitize_label(req.prefix.as_deref().unwrap_or(&name));
  if prefix.is_empty() {
    return Err(DomainError::invalid(
      "Enter a name for the address, eg. 'grafana'.",
    ));
  }
  let hostname =
    format!("{prefix}.{}", choose_zone(req.zone.as_deref()).await?);

  let port = if req.port != 0 {
    req.port
  } else {
    let ports =
      service_ports(&resolved.target, &resolved.service, &server.id)
        .await;
    choose_port(&ports).unwrap_or(resolved.detected_port)
  };
  if port == 0 {
    return Err(DomainError::invalid(
      "No published port found. Start the service with a published port, or pick the port to use.",
    ));
  }

  // 1. Connection (skipped quickly when it is already up).
  tunnel::enable(
    EnableRequest {
      server: server.id.clone(),
      account: None,
      image: None,
    },
    user,
  )
  .await?;

  // 2. Address.
  progress.set(ExposeStage::Configuring);
  let added = add_service_domain(
    AddRequest {
      target: resolved.target,
      service: resolved.service,
      hostname,
      port,
      mode: DomainMode::Tunnel,
      adopt_conflicts: req.adopt_conflicts,
    },
    user,
  )
  .await?;
  if !added.conflicts.is_empty() || added.error.is_some() {
    return Ok(ExposeServiceResult {
      url: None,
      conflicts: added.conflicts,
      error: added.error,
    });
  }
  wait_connected(&server.id).await;
  Ok(ExposeServiceResult {
    url: added
      .domain
      .map(|d| format!("https://{}", d.config.hostname)),
    ..Default::default()
  })
}

/// Remove every public address of the service. Remote records Komodo
/// created are cleaned up by the Domain delete itself.
pub async fn unexpose(
  target: &ResourceTarget,
  service: &str,
  user: &User,
) -> Result<Vec<String>, DomainError> {
  let resolved = resolve_target(target, service)?;
  let domains: Vec<Domain> =
    find_collect(&db_client().domains, doc! {}, None)
      .await
      .map_err(DomainError::internal)?;
  let mut removed = Vec::new();
  for d in domains.into_iter().filter(|d| {
    d.config.target.as_ref() == Some(&resolved.target)
      && d.config.service == resolved.service
  }) {
    resource::delete::<Domain>(&d.id, user).await.map_err(|e| {
      DomainError::invalid(format!(
        "Could not remove '{}': {e:#}. Try again, or delete it from Settings -> Domains.",
        d.config.hostname
      ))
    })?;
    removed.push(d.config.hostname);
  }
  Ok(removed)
}
