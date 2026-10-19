//! One-click tunnel for a Server: open the tunnel at the provider, bind it
//! to the Server in the account params, and run the connector there as a
//! normal Komodo Deployment (so permissions, Updates and logs all apply).
//!
//! Vendor neutral: everything provider specific lives behind
//! [ProvisionProvider].
//!
//! The connector token is stored in a secret Variable and the Deployment
//! only references it (`TUNNEL_TOKEN=[[VAR]]`). See doc/domains-v2-design.md
//! for the trade-offs.

use std::time::Duration;

use database::mungos::{
  by_id::find_one_by_id, find::find_collect, mongodb::bson::doc,
  mongodb::bson::oid::ObjectId,
};
use komodo_client::{
  api::execute::Deploy,
  entities::{
    deployment::{
      _PartialDeploymentConfig, Deployment, DeploymentImage,
      DeploymentState, RestartMode,
    },
    domain::Domain,
    domain_account::{ServerTunnel, ServerTunnelState},
    server::ServerState,
    update::UpdateStatus,
    user::User,
    variable::Variable,
  },
};
use serde_json::Value;

use crate::{
  api::execute::{ExecuteRequest, ExecutionResult, inner_handler},
  helpers::query::{get_cached_server_state, get_deployment_state},
  resource,
  state::{all_resources_cache, db_client},
};

use super::{
  accounts::{self, ResolvedAccount},
  error::DomainError,
  lock::lock_accounts,
  prepare::{find_by_id_or_name, server_of},
  provider::{
    ConnectorSpec, ProviderSet, ProvisionProvider, ServerRef,
    TunnelHandle,
  },
};

pub struct EnableRequest {
  pub server: String,
  pub account: Option<String>,
  pub image: Option<String>,
}

/// An account which can open tunnels, with its providers.
struct Candidate {
  account: ResolvedAccount,
  set: ProviderSet,
}

impl Candidate {
  fn provision(&self) -> Result<&dyn ProvisionProvider, DomainError> {
    self.set.provision.as_deref().ok_or_else(|| {
      DomainError::invalid(format!(
        "Account '{}' cannot open connections by itself",
        self.account.name
      ))
    })
  }
}

fn resolve_server(
  id_or_name: &str,
) -> Result<ServerRef, DomainError> {
  let all = all_resources_cache().load();
  find_by_id_or_name(&all.servers, id_or_name, |s| &s.name)
    .map(|s| ServerRef {
      id: s.id.clone(),
      name: s.name.clone(),
    })
    .ok_or_else(|| {
      DomainError::invalid(format!(
        "Server '{id_or_name}' does not exist"
      ))
    })
}

fn deployment_name(server: &ServerRef) -> String {
  format!("cloudflared-{}", server.name)
}

fn tunnel_name(server: &ServerRef) -> String {
  format!("komodo-{}", server.name)
}

/// Secret Variable which holds the connector token.
fn variable_name(server: &ServerRef) -> String {
  let id: String = server
    .name
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() {
        c.to_ascii_uppercase()
      } else {
        '_'
      }
    })
    .collect();
  format!("TUNNEL_TOKEN_{id}")
}

/// Accounts which can open tunnels (providers built, no network).
async fn candidates() -> Result<Vec<Candidate>, DomainError> {
  Ok(
    accounts::list_all()
      .await?
      .into_iter()
      .filter_map(|account| {
        let set = account.providers().ok()?;
        set
          .provision
          .is_some()
          .then_some(Candidate { account, set })
      })
      .collect(),
  )
}

fn bound<'a>(
  candidates: &'a [Candidate],
  server: &ServerRef,
) -> Option<(&'a Candidate, TunnelHandle)> {
  candidates.iter().find_map(|c| {
    c.set
      .provision
      .as_deref()?
      .registered(server)
      .map(|handle| (c, handle))
  })
}

async fn find_deployment(
  server: &ServerRef,
) -> Result<Option<Deployment>, DomainError> {
  let found = db_client()
    .deployments
    .find_one(doc! { "name": deployment_name(server) })
    .await
    .map_err(|e| DomainError::internal(format!("{e:#}")))?;
  match found {
    Some(d) if d.config.server_id == server.id => Ok(Some(d)),
    Some(d) => Err(DomainError::invalid(format!(
      "A Deployment named '{}' already exists on another server. Rename or delete it, then try again.",
      d.name
    ))),
    None => Ok(None),
  }
}

/// Domains which use the tunnel, by name.
async fn domains_using(
  account: &str,
  handle: &TunnelHandle,
  scope: Option<&str>,
  server: &ServerRef,
) -> Result<Vec<String>, DomainError> {
  let domains: Vec<Domain> = find_collect(
    &db_client().domains,
    doc! { "config.ingress.account": account },
    None,
  )
  .await
  .map_err(|e| DomainError::internal(format!("{e:#}")))?;
  Ok(
    domains
      .into_iter()
      .filter(|d| {
        let explicit = d.config.ingress.tunnel == handle.name;
        let owned = scope.is_some_and(|scope| {
          d.info.owned.iter().any(|o| o.scope == scope)
        });
        let implicit = d.config.ingress.tunnel.is_empty()
          && d
            .config
            .target
            .as_ref()
            .and_then(|t| server_of(t).ok().flatten())
            .is_some_and(|s| s.id == server.id);
        explicit || owned || implicit
      })
      .map(|d| d.name)
      .collect(),
  )
}

fn scope_of(
  set: &ProviderSet,
  handle: &TunnelHandle,
) -> Option<String> {
  set
    .ingress
    .as_ref()?
    .select(Some(&handle.name), None)
    .ok()
    .map(|t| t.id)
}

/// Current state. Reads the provider status (one GET) when bound.
pub async fn status(
  server: &str,
) -> Result<ServerTunnel, DomainError> {
  status_of(&resolve_server(server)?).await
}

async fn status_of(
  server: &ServerRef,
) -> Result<ServerTunnel, DomainError> {
  let candidates = candidates().await?;
  let names: Vec<String> =
    candidates.iter().map(|c| c.account.name.clone()).collect();
  let mut out = ServerTunnel {
    accounts: names.clone(),
    ..Default::default()
  };
  let found = bound(&candidates, server);
  let chosen = found
    .as_ref()
    .map(|(c, _)| *c)
    .or_else(|| (candidates.len() == 1).then(|| &candidates[0]));
  if let Some(c) = chosen {
    out.account = Some(c.account.name.clone());
    out.image = c.provision()?.default_image();
  }
  let deployment = find_deployment(server).await.unwrap_or(None);
  if let Some(d) = &deployment {
    out.deployment = Some(d.name.clone());
    out.deployment_id = Some(d.id.clone());
    out.deployment_state = get_deployment_state(&d.id).await.ok();
  }
  let Some((candidate, handle)) = found else {
    return Ok(out);
  };
  out.tunnel_name = Some(handle.name.clone());
  out.managed = handle.managed;
  let scope = scope_of(&candidate.set, &handle);
  out.domains = domains_using(
    &candidate.account.name,
    &handle,
    scope.as_deref(),
    server,
  )
  .await?;
  match candidate.provision()?.status(&handle).await {
    Ok(status) => out.remote_status = Some(status),
    Err(e) => {
      e.log("GetServerTunnel status");
      out.warning = Some(format!(
        "Could not read the connection status: {}",
        e.message
      ));
    }
  }
  let connector_up = matches!(
    out.deployment_state,
    Some(DeploymentState::Running | DeploymentState::Deploying)
  );
  out.state = match out.remote_status.as_deref() {
    _ if out.deployment_state == Some(DeploymentState::Deploying) => {
      ServerTunnelState::Starting
    }
    Some("healthy") => ServerTunnelState::Healthy,
    Some("degraded") => ServerTunnelState::Degraded,
    Some("down") => ServerTunnelState::Down,
    _ if connector_up => ServerTunnelState::Starting,
    _ => ServerTunnelState::Inactive,
  };
  if deployment.is_none() && out.warning.is_none() {
    out.warning = Some(
      "The connection is registered but nothing runs on this server yet. Make a service public again to finish setting it up."
        .into(),
    );
  }
  Ok(out)
}

async fn save_params(
  account: &ResolvedAccount,
  params: &Value,
) -> Result<(), DomainError> {
  (account.factory()?.validate_params)(params)?;
  let oid = ObjectId::parse_str(&account.id)
    .map_err(DomainError::internal)?;
  let bson =
    database::bson::to_bson(params).map_err(DomainError::internal)?;
  db_client()
    .domain_accounts
    .update_one(
      doc! { "_id": oid },
      doc! { "$set": { "params": bson } },
    )
    .await
    .map_err(|e| DomainError::internal(format!("{e:#}")))?;
  Ok(())
}

fn require_db_account(
  account: &ResolvedAccount,
) -> Result<(), DomainError> {
  if account.id.is_empty() {
    return Err(DomainError::invalid(format!(
      "Account '{}' is declared in the Core config file and is read only. Create an account in Settings -> Domains instead.",
      account.name
    )));
  }
  Ok(())
}

/// Write the token to the secret Variable (create or replace).
/// Written straight to the db: the Update log of `CreateVariable`
/// would print the value.
async fn store_token(
  name: &str,
  token: &str,
) -> Result<(), DomainError> {
  let col = &db_client().variables;
  let internal = |e: database::mungos::mongodb::error::Error| {
    DomainError::internal(format!("{e:#}"))
  };
  if col
    .find_one(doc! { "name": name })
    .await
    .map_err(internal)?
    .is_some()
  {
    col
      .update_one(
        doc! { "name": name },
        doc! { "$set": { "value": token, "is_secret": true } },
      )
      .await
      .map_err(internal)?;
  } else {
    col
      .insert_one(&Variable {
        name: name.to_string(),
        value: token.to_string(),
        description: "Cloudflare Tunnel connector token. Managed by Komodo, do not edit.".into(),
        is_secret: true,
      })
      .await
      .map_err(internal)?;
  }
  Ok(())
}

async fn remove_token(name: &str) -> Result<(), DomainError> {
  db_client()
    .variables
    .delete_one(doc! { "name": name })
    .await
    .map_err(|e| DomainError::internal(format!("{e:#}")))?;
  Ok(())
}

fn api_error(e: impl std::fmt::Debug) -> DomainError {
  DomainError::invalid(format!("{e:?}"))
}

/// How long one connector Deploy may take (image pull included).
const DEPLOY_TIMEOUT: Duration = Duration::from_secs(300);

/// Wait for an Update to finish, then say whether it succeeded.
async fn update_succeeded(id: &str) -> Result<bool, DomainError> {
  let deadline = tokio::time::Instant::now() + DEPLOY_TIMEOUT;
  loop {
    let update = find_one_by_id(&db_client().updates, id)
      .await
      .map_err(|e| DomainError::internal(format!("{e:#}")))?;
    if let Some(update) = update
      && update.status == UpdateStatus::Complete
    {
      return Ok(update.success);
    }
    if tokio::time::Instant::now() >= deadline {
      return Ok(false);
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
  }
}

/// Create the connector Deployment (or fix its image), deploy it and wait
/// for the Deploy to finish. `false` = the container did not start, which
/// is what a blocked registry looks like (the pull fails).
async fn deploy_connector(
  server: &ServerRef,
  spec: &ConnectorSpec,
  existing: Option<Deployment>,
  user: &User,
) -> Result<bool, DomainError> {
  let var = variable_name(server);
  let [(env_key, token)] = spec.env.as_slice() else {
    return Err(DomainError::internal("unexpected connector env"));
  };
  store_token(&var, token).await?;
  let image = DeploymentImage::Image {
    image: spec.image.clone(),
  };
  let name = match existing {
    Some(d) => {
      if d.config.image != image {
        resource::update::<Deployment>(
          &d.id,
          _PartialDeploymentConfig {
            image: Some(image),
            ..Default::default()
          },
          user,
        )
        .await
        .map_err(api_error)?;
      }
      d.name
    }
    None => {
      let name = deployment_name(server);
      resource::create::<Deployment>(
        &name,
        _PartialDeploymentConfig {
          server_id: Some(server.id.clone()),
          image: Some(image),
          network: Some("host".into()),
          command: Some(spec.command.clone()),
          restart: Some(RestartMode::UnlessStopped),
          environment: Some(format!("{env_key}=[[{var}]]")),
          ..Default::default()
        },
        None,
        user,
      )
      .await
      .map_err(api_error)?;
      name
    }
  };
  let ExecutionResult::Single(update) = inner_handler(
    ExecuteRequest::Deploy(Deploy {
      deployment: name,
      stop_signal: None,
      stop_time: None,
    }),
    user.clone(),
  )
  .await
  .map_err(api_error)?
  else {
    return Err(DomainError::internal("unexpected Deploy result"));
  };
  update_succeeded(&update.id).await
}

/// Preferred image first, then the fallbacks, without repeats.
fn image_candidates(
  preferred: &str,
  fallbacks: &[String],
) -> Vec<String> {
  let mut out = vec![preferred.to_string()];
  for image in fallbacks {
    if !out.contains(image) {
      out.push(image.clone());
    }
  }
  out
}

pub async fn enable(
  req: EnableRequest,
  user: &User,
) -> Result<ServerTunnel, DomainError> {
  let server = resolve_server(&req.server)?;
  let candidates = candidates().await?;
  let found = bound(&candidates, &server);
  let candidate = match &found {
    Some((c, _)) => {
      if req.account.as_deref().is_some_and(|a| a != c.account.name) {
        return Err(DomainError::invalid(format!(
          "This server's connection belongs to account '{}', another account cannot be used.",
          c.account.name
        )));
      }
      *c
    }
    None => {
      let pool: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| {
          req.account.as_deref().is_none_or(|a| a == c.account.name)
        })
        .collect();
      match pool.as_slice() {
        [one] => *one,
        [] => {
          return Err(DomainError::invalid(
            "No account can open a public connection yet. Add a Cloudflare account in Settings -> Domains first.",
          ));
        }
        many => {
          return Err(DomainError::invalid(format!(
            "Several accounts can open a public connection ({}). Pass the one to use.",
            many
              .iter()
              .map(|c| c.account.name.as_str())
              .collect::<Vec<_>>()
              .join(", ")
          )));
        }
      }
    }
  };
  require_db_account(&candidate.account)?;
  let provision = candidate.provision()?;
  let _guard = lock_accounts([candidate.account.name.as_str()]).await;

  // Already up: nothing to do.
  let deployment = find_deployment(&server).await?;
  if let (Some(_), Some(d)) = (&found, &deployment) {
    let state = get_deployment_state(&d.id).await.unwrap_or_default();
    if matches!(
      state,
      DeploymentState::Running | DeploymentState::Deploying
    ) {
      return status_of(&server).await;
    }
  }

  let state = get_cached_server_state(&server.id).await;
  if state != ServerState::Ok {
    return Err(DomainError::invalid(format!(
      "Server '{}' is not reachable (state: {state:?}), so the connection cannot be set up on it. Bring the server back online and try again.",
      server.name
    )));
  }

  let preferred = req
    .image
    .as_deref()
    .map(str::trim)
    .filter(|i| !i.is_empty())
    .map(str::to_string)
    .unwrap_or_else(|| provision.default_image());
  let candidates =
    image_candidates(&preferred, &provision.fallback_images());

  // `stored`: the image the account remembers right now.
  let (handle, mut params, stored, mut first_spec) = match found {
    Some((_, handle)) => (
      handle,
      candidate.account.params.clone(),
      provision.default_image(),
      None,
    ),
    None => {
      let created = provision
        .provision(&tunnel_name(&server), &candidates[0])
        .await?;
      let params = provision.register(
        &candidate.account.params,
        &created.handle,
        &server,
        &candidates[0],
      );
      if let Err(e) = save_params(&candidate.account, &params).await {
        // Registration failed: don't leave an unknown tunnel behind.
        if let Err(cleanup) = provision.destroy(&created.handle).await
        {
          cleanup.log("rollback created tunnel");
        }
        return Err(e);
      }
      (
        created.handle,
        params,
        candidates[0].clone(),
        Some(created.connector),
      )
    }
  };

  // A failed Deploy usually means the image could not be pulled (the
  // registry is blocked from this host): move on to the next source.
  for image in &candidates {
    let spec = match first_spec.take() {
      Some(spec) => spec,
      None => provision.connector(&handle, image).await?,
    };
    let existing = find_deployment(&server).await?;
    if deploy_connector(&server, &spec, existing, user).await? {
      if *image != stored {
        params = provision.remember_image(&params, image);
        save_params(&candidate.account, &params).await?;
      }
      return status_of(&server).await;
    }
    warn!(
      "cloudflared did not start on '{}' with image '{image}', trying the next source",
      server.name
    );
  }
  Err(DomainError::invalid(format!(
    "The connection could not be started on server '{0}': cloudflared failed to start from every image source. Check that '{0}' has internet access and see the Update log of Deployment '{1}', then try again.",
    server.name,
    deployment_name(&server)
  )))
}

pub async fn disable(
  server: &str,
  delete_remote: bool,
  user: &User,
) -> Result<ServerTunnel, DomainError> {
  let server = resolve_server(server)?;
  let candidates = candidates().await?;
  let Some((candidate, handle)) = bound(&candidates, &server) else {
    return status_of(&server).await;
  };
  if !handle.managed {
    return Err(DomainError::invalid(
      "This connection was not created by Komodo. Remove it by hand in the account params in Settings -> Domains.",
    ));
  }
  require_db_account(&candidate.account)?;
  let provision = candidate.provision()?;
  let _guard = lock_accounts([candidate.account.name.as_str()]).await;

  let scope = scope_of(&candidate.set, &handle);
  let used = domains_using(
    &candidate.account.name,
    &handle,
    scope.as_deref(),
    &server,
  )
  .await?;
  if !used.is_empty() {
    return Err(DomainError::invalid(format!(
      "Still in use by public addresses: {}. Stop public access of those services first.",
      used.join(", ")
    )));
  }

  if let Some(d) = find_deployment(&server).await? {
    resource::delete::<Deployment>(&d.id, user)
      .await
      .map_err(api_error)?;
  }
  remove_token(&variable_name(&server)).await?;
  if delete_remote {
    provision.destroy(&handle).await?;
  }
  let params =
    provision.unregister(&candidate.account.params, &handle);
  save_params(&candidate.account, &params).await?;
  status_of(&server).await
}

#[cfg(test)]
mod tests {
  use super::image_candidates;

  #[test]
  fn candidates_start_with_preferred_and_skip_repeats() {
    let fallbacks =
      ["official:1", "mirror-a:1", "mirror-b:1"].map(String::from);
    // A remembered mirror is tried first, the official image is still
    // tried before the other mirror.
    assert_eq!(
      image_candidates("mirror-a:1", &fallbacks),
      ["mirror-a:1", "official:1", "mirror-b:1"]
    );
    assert_eq!(image_candidates("custom:1", &fallbacks).len(), 4);
  }
}
