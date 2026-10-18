//! Fail-closed file publication. Directories must be private and exclusively
//! managed by Komodo; no remote request can select a filesystem path.
use super::Args;
use crate::config::periphery_config;
use anyhow::{Context, Result, ensure};
use mogh_resolver::Resolve;
use periphery_client::api::domain::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
  fs::{self, File, OpenOptions},
  io::{Read, Write},
  os::{
    fd::{AsRawFd, FromRawFd},
    unix::{
      ffi::OsStrExt,
      fs::{OpenOptionsExt, PermissionsExt},
    },
  },
  path::{Component, Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
struct State {
  fence: u64,
  fingerprint: Option<String>,
  #[serde(default)]
  pending: bool,
  #[serde(default)]
  pending_fingerprint: Option<String>,
  #[serde(default)]
  last_valid_fingerprint: Option<String>,
}

fn identifier(value: &str) -> Result<()> {
  ensure!(
    !value.is_empty()
      && value.len() <= 128
      && value
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
    "InvalidDomainIdentifier"
  );
  Ok(())
}
fn digest(bytes: &[u8]) -> String {
  Sha256::digest(bytes)
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect()
}

fn directory(ingress: &str) -> Result<PathBuf> {
  identifier(ingress)?;
  ensure!(cfg!(target_os = "linux"), "UnsupportedDomainPlatform");
  let config = periphery_config();
  ensure!(config.domain_enabled, "DomainFeatureDisabled");
  config
    .domain_ingress_directories
    .get(ingress)
    .cloned()
    .context("IngressNotAllowed")
}

// Lock the directory inode, not a replaceable lock file. Held through fsync.
fn lock_directory(path: &Path) -> Result<File> {
  ensure!(path.is_absolute(), "DomainDirectoryMustBeAbsolute");
  // Walk from the root through directory descriptors. O_NOFOLLOW at every
  // component closes the check/open race of symlink_metadata + open(path).
  let mut file = File::open("/")?;
  for part in path.components() {
    let Component::Normal(name) = part else {
      ensure!(
        matches!(part, Component::RootDir | Component::CurDir),
        "InvalidDomainDirectory"
      );
      continue;
    };
    let name = std::ffi::CString::new(name.as_bytes())?;
    // SAFETY: directory fd and NUL-terminated component remain valid.
    let fd = unsafe {
      libc::openat(
        file.as_raw_fd(),
        name.as_ptr(),
        libc::O_RDONLY
          | libc::O_DIRECTORY
          | libc::O_NOFOLLOW
          | libc::O_CLOEXEC,
      )
    };
    if fd < 0 {
      return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: openat returned a new owned fd.
    file = unsafe { File::from_raw_fd(fd) };
  }
  ensure!(
    file.metadata()?.permissions().mode() & 0o022 == 0,
    "DomainDirectoryMustBePrivate"
  );
  // SAFETY: valid directory fd remains alive for the entire operation.
  ensure!(
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0,
    "DomainDirectoryLockFailed"
  );
  Ok(file)
}
fn read(path: &Path) -> Result<Option<Vec<u8>>> {
  let mut file = match OpenOptions::new()
    .read(true)
    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
    .open(path)
  {
    Ok(file) => file,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
      return Ok(None);
    }
    Err(error) => return Err(error.into()),
  };
  ensure!(file.metadata()?.is_file(), "DomainFileNotRegular");
  ensure!(
    file.metadata()?.len() <= 1024 * 1024,
    "DomainFileTooLarge"
  );
  let mut bytes = Vec::new();
  file.read_to_end(&mut bytes)?;
  Ok(Some(bytes))
}
fn atomic(path: &Path, bytes: &[u8], directory: &File) -> Result<()> {
  let temp = path
    .with_file_name(format!(".komodo-{}.tmp", uuid::Uuid::new_v4()));
  let result = (|| {
    let mut file = OpenOptions::new()
      .write(true)
      .create_new(true)
      .mode(0o600)
      .custom_flags(libc::O_NOFOLLOW)
      .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    directory.sync_all()?;
    Ok(())
  })();
  if result.is_err() {
    let _ = fs::remove_file(&temp);
  }
  result
}
fn render(domain: &str, route: &DomainRoute) -> Result<Vec<u8>> {
  identifier(domain)?;
  let host = &route.hostname;
  ensure!(
    host.len() <= 253
      && host.contains('.')
      && host.split('.').all(|label| !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label.bytes().all(|c| c.is_ascii_lowercase()
          || c.is_ascii_digit()
          || c == b'-')),
    "InvalidHostname"
  );
  ensure!(route.backend_url.len() <= 8192, "InvalidBackendUrl");
  ensure!(route.entrypoints.len() <= 16, "InvalidEntrypoints");
  let url = url::Url::parse(&route.backend_url)
    .context("InvalidBackendUrl")?;
  ensure!(
    matches!(url.scheme(), "http" | "https")
      && url.host_str().is_some()
      && url.username().is_empty()
      && url.password().is_none()
      && url.fragment().is_none(),
    "InvalidBackendUrl"
  );
  ensure!(!route.entrypoints.is_empty(), "MissingEntrypoint");
  for entry in &route.entrypoints {
    identifier(entry)?;
  }
  if let Some(resolver) = &route.certificate_resolver {
    identifier(resolver)?;
  }
  ensure!(
    route.require_https || route.certificate_resolver.is_none(),
    "InvalidTlsPolicy"
  );
  let name = format!("komodo-{domain}");
  let mut router = json!({"rule": format!("Host(`{host}`)"), "entryPoints": route.entrypoints, "service": name});
  if route.require_https {
    router["tls"] = match &route.certificate_resolver {
      Some(resolver) => json!({"certResolver": resolver}),
      None => json!({}),
    };
  }
  // JSON syntax is valid YAML; .yml makes Traefik file provider load it.
  Ok(serde_json::to_vec_pretty(
    &json!({"http": {"routers": {name.clone(): router}, "services": {name: {"loadBalancer": {"servers": [{"url": route.backend_url}]}}}}}),
  )?)
}

// File-provider directories are dedicated. Refuse foreign dynamic configuration
// and a hostname already published for a different binding.
fn check_routes(
  path: &Path,
  domain: &str,
  hostname: &str,
) -> Result<()> {
  for entry in fs::read_dir(path)? {
    let entry = entry?;
    let kind = entry.file_type()?;
    ensure!(
      !kind.is_symlink() && !kind.is_dir(),
      "ForeignIngressConfiguration"
    );
    let name = entry.file_name();
    let name =
      name.to_str().context("ForeignIngressConfiguration")?;
    if !name.ends_with(".yml")
      && !name.ends_with(".yaml")
      && !name.ends_with(".toml")
    {
      continue;
    }
    let binding = name
      .strip_prefix("komodo-")
      .and_then(|name| name.strip_suffix(".yml"))
      .context("ForeignIngressConfiguration")?;
    identifier(binding)?;
    if binding == domain {
      continue;
    }
    let bytes =
      read(&entry.path())?.context("ForeignIngressConfiguration")?;
    let state: State = serde_json::from_slice(
      &read(&path.join(format!(".komodo-{binding}.state")))?
        .context("ForeignIngressConfiguration")?,
    )?;
    ensure!(
      Some(digest(&bytes)) == state.fingerprint,
      "ForeignIngressConfiguration"
    );
    let config: serde_json::Value = serde_json::from_slice(&bytes)
      .context("ForeignIngressConfiguration")?;
    let routers = config["http"]["routers"]
      .as_object()
      .context("ForeignIngressConfiguration")?;
    let rule = format!("Host(`{hostname}`)");
    ensure!(
      !routers
        .values()
        .any(|router| router["rule"].as_str() == Some(rule.as_str())),
      "HostnameConflict"
    );
  }
  Ok(())
}

fn clock_ms() -> Result<u64> {
  Ok(
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)?
      .as_millis()
      .try_into()?,
  )
}
fn check_deadline(deadline: u64) -> Result<()> {
  let now = clock_ms()?;
  ensure!(
    deadline > now && deadline <= now.saturating_add(60_000),
    "DomainLeaseExpired"
  );
  Ok(())
}
#[cfg(test)]
fn operate(
  path: &Path,
  domain: &str,
  fence: u64,
  expected: Option<String>,
  mutation: Option<Mutation>,
) -> Result<DomainRouteObservation> {
  operate_before(
    path,
    domain,
    fence,
    expected,
    mutation,
    clock_ms()? + 60_000,
  )
}

enum Mutation {
  Apply(DomainRoute),
  Remove,
}
fn operate_before(
  path: &Path,
  domain: &str,
  fence: u64,
  expected: Option<String>,
  mutation: Option<Mutation>,
  deadline: u64,
) -> Result<DomainRouteObservation> {
  check_deadline(deadline)?;
  identifier(domain)?;
  let directory = lock_directory(path)?;
  check_deadline(deadline)?;
  // Anchor all following lookups to the locked inode, even if an administrator
  // concurrently renames its parent. This RPC is Linux-only (Periphery images).
  ensure!(cfg!(target_os = "linux"), "UnsupportedDomainPlatform");
  let anchored =
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
  let path = anchored.as_path();
  let route_path = path.join(format!("komodo-{domain}.yml"));
  let state_path = path.join(format!(".komodo-{domain}.state"));
  let mut state: State = match read(&state_path)? {
    Some(bytes) => {
      serde_json::from_slice(&bytes).context("InvalidDomainState")?
    }
    None => State::default(),
  };
  let current = read(&route_path)?.map(|bytes| digest(&bytes));
  let Some(mutation) = mutation else {
    return Ok(DomainRouteObservation {
      exists: current.is_some(),
      fingerprint: current,
      fencing_token: state.fence,
      load_verified: false,
      load_error: None,
      rolled_back: false,
    });
  };
  ensure!(fence > 0 && fence >= state.fence, "StaleFencingToken");
  let desired = match mutation {
    Mutation::Apply(route) => {
      check_routes(path, domain, &route.hostname)?;
      Some(render(domain, &route)?)
    }
    Mutation::Remove => None,
  };
  let desired_digest = desired.as_ref().map(|bytes| digest(bytes));
  let recovering_intent =
    state.pending && state.pending_fingerprint == desired_digest;
  // Recover an interrupted rename/remove by observing both persisted outcomes.
  if state.pending {
    if current == state.pending_fingerprint {
      state.fingerprint = state.pending_fingerprint.clone();
    } else {
      ensure!(current == state.fingerprint, "ExternalDrift");
    }
    state.pending = false;
    state.pending_fingerprint = None;
  }
  ensure!(
    fence > state.fence
      || desired_digest == state.fingerprint
      || recovering_intent,
    "FencingTokenReused"
  );
  // A replay can only succeed if the on-disk object is still explicitly owned.
  if current != desired_digest || current != state.fingerprint {
    ensure!(current == expected, "ExternalDrift");
    ensure!(current == state.fingerprint, "UnownedRouteConflict");
  }
  check_deadline(deadline)?;
  // Persist fencing even for no-op writes; old tasks cannot subsequently mutate.
  state.fence = fence;
  if current != desired_digest {
    match desired {
      Some(bytes) => {
        // Persist intent before publication for recovery of response loss.
        state.pending = true;
        state.pending_fingerprint = desired_digest.clone();
        atomic(
          &state_path,
          &serde_json::to_vec(&state)?,
          &directory,
        )?;
        check_deadline(deadline)?;
        atomic(&route_path, &bytes, &directory)?;
      }
      None => {
        state.pending = true;
        state.pending_fingerprint = None;
        atomic(
          &state_path,
          &serde_json::to_vec(&state)?,
          &directory,
        )?;
        check_deadline(deadline)?;
        fs::remove_file(&route_path)?;
        directory.sync_all()?;
      }
    }
  }
  check_deadline(deadline)?;
  state.fingerprint = desired_digest.clone();
  state.pending = false;
  state.pending_fingerprint = None;
  atomic(&state_path, &serde_json::to_vec(&state)?, &directory)?;
  Ok(DomainRouteObservation {
    exists: desired_digest.is_some(),
    fingerprint: desired_digest,
    fencing_token: fence,
    load_verified: false,
    load_error: None,
    rolled_back: false,
  })
}

#[derive(Debug, PartialEq)]
enum Loaded {
  Verified,
  Pending,
  Failed,
}

fn observation_url(ingress: &str) -> Result<Option<url::Url>> {
  periphery_config()
    .domain_ingress_observation_urls
    .get(ingress)
    .map(|base| {
      let mut url = url::Url::parse(base)
        .context("InvalidObservationEndpoint")?;
      ensure!(
        matches!(url.scheme(), "http" | "https")
          && url.host_str().is_some()
          && url.username().is_empty()
          && url.password().is_none()
          && url.query().is_none()
          && url.fragment().is_none(),
        "InvalidObservationEndpoint"
      );
      url.set_path(&format!(
        "{}/api/rawdata",
        url.path().trim_end_matches('/')
      ));
      Ok(url)
    })
    .transpose()
}

fn verify_rawdata(
  domain: &str,
  config: &[u8],
  raw: &serde_json::Value,
) -> Result<Loaded> {
  let expected: serde_json::Value = serde_json::from_slice(config)?;
  let name = format!("komodo-{domain}");
  let qualified = format!("{name}@file");
  let router = &raw["routers"][&qualified];
  let service = &raw["services"][&qualified];
  let desired = &expected["http"]["routers"][&name];
  let backend =
    &expected["http"]["services"][&name]["loadBalancer"]["servers"];
  let service_ref = router["service"].as_str();
  // An old configuration can still be served during watch propagation. Only
  // attribute errors to this write after its declared configuration matches.
  if router.is_null()
    || router["rule"] != desired["rule"]
    || router["entryPoints"] != desired["entryPoints"]
    || !matches!(service_ref, Some(value) if value == name || value == qualified)
    || !desired["tls"].is_null() != !router["tls"].is_null()
    || desired["tls"]["certResolver"] != router["tls"]["certResolver"]
  {
    return Ok(Loaded::Pending);
  }
  let failed = |value: &serde_json::Value| {
    value["status"].as_str() == Some("disabled")
      || ["error", "errors"].iter().any(|key| {
        value[*key]
          .as_array()
          .is_some_and(|errors| !errors.is_empty())
      })
  };
  if failed(router) {
    return Ok(Loaded::Failed);
  }
  if service.is_null()
    || service["loadBalancer"]["servers"] != *backend
  {
    return Ok(Loaded::Pending);
  }
  if failed(service) {
    return Ok(Loaded::Failed);
  }
  if router["status"].as_str() != Some("enabled")
    || service["status"].as_str() != Some("enabled")
  {
    return Ok(Loaded::Pending);
  }
  Ok(Loaded::Verified)
}

async fn fetch_loaded(
  url: &url::Url,
  domain: &str,
  config: &[u8],
) -> Result<Loaded> {
  let client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .no_proxy()
    .timeout(std::time::Duration::from_secs(2))
    .build()
    .context("ObservationUnavailable")?;
  let mut response = client
    .get(url.clone())
    .send()
    .await
    .map_err(|_| anyhow::anyhow!("ObservationUnavailable"))?;
  ensure!(response.status().is_success(), "ObservationUnavailable");
  ensure!(
    response.content_length().unwrap_or(0) <= 1024 * 1024,
    "ObservationTooLarge"
  );
  let mut bytes = Vec::new();
  while let Some(chunk) = response
    .chunk()
    .await
    .map_err(|_| anyhow::anyhow!("ObservationUnavailable"))?
  {
    ensure!(
      bytes.len() + chunk.len() <= 1024 * 1024,
      "ObservationTooLarge"
    );
    bytes.extend_from_slice(&chunk);
  }
  let raw = serde_json::from_slice(&bytes)
    .map_err(|_| anyhow::anyhow!("InvalidObservationResponse"))?;
  verify_rawdata(domain, config, &raw)
}

fn owned_snapshot(
  path: &Path,
  domain: &str,
) -> Result<Option<Vec<u8>>> {
  owned_snapshot_at_fence(path, domain, None)
}

fn owned_snapshot_at_fence(
  path: &Path,
  domain: &str,
  fence: Option<u64>,
) -> Result<Option<Vec<u8>>> {
  identifier(domain)?;
  let directory = lock_directory(path)?;
  let base =
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
  let bytes = read(&base.join(format!("komodo-{domain}.yml")))?;
  if let Some(bytes) = &bytes {
    let state_path = base.join(format!(".komodo-{domain}.state"));
    let mut state: State = serde_json::from_slice(
      &read(&state_path)?.context("UnownedRouteConflict")?,
    )?;
    let current = digest(bytes);
    if let Some(fence) = fence {
      ensure!(fence > 0 && fence >= state.fence, "StaleFencingToken");
      // A prior task may have published successfully and died before its state
      // commit. Settle only that exact durable intent before readiness/backup
      // checks; otherwise a loaded new file would be mistaken for foreign drift.
      if state.pending
        && state.pending_fingerprint.as_deref()
          == Some(current.as_str())
      {
        state.fingerprint = state.pending_fingerprint.take();
        state.pending = false;
        atomic(
          &state_path,
          &serde_json::to_vec(&state)?,
          &directory,
        )?;
      }
    }
    ensure!(
      state.fingerprint.as_deref() == Some(current.as_str())
        || (state.pending
          && state.pending_fingerprint.as_deref()
            == Some(current.as_str())),
      "ExternalDrift"
    );
  }
  Ok(bytes)
}

fn rollback(
  path: &Path,
  domain: &str,
  fence: u64,
  failed: &str,
  previous: Option<Vec<u8>>,
  deadline: u64,
) -> Result<DomainRouteObservation> {
  identifier(domain)?;
  check_deadline(deadline)?;
  let directory = lock_directory(path)?;
  check_deadline(deadline)?;
  let base =
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
  let file = base.join(format!("komodo-{domain}.yml"));
  let state_path = base.join(format!(".komodo-{domain}.state"));
  let mut state: State = serde_json::from_slice(
    &read(&state_path)?.context("UnownedRouteConflict")?,
  )?;
  ensure!(
    state.fence == fence
      && state.fingerprint.as_deref() == Some(failed)
      && read(&file)?.as_ref().map(|bytes| digest(bytes)).as_deref()
        == Some(failed),
    "RollbackConflict"
  );
  state.pending = true;
  state.pending_fingerprint =
    previous.as_ref().map(|bytes| digest(bytes));
  atomic(&state_path, &serde_json::to_vec(&state)?, &directory)?;
  check_deadline(deadline)?;
  if let Some(bytes) = previous {
    atomic(&file, &bytes, &directory)?;
  } else {
    fs::remove_file(&file)?;
    directory.sync_all()?;
  }
  state.fingerprint = state.pending_fingerprint.take();
  state.pending = false;
  atomic(&state_path, &serde_json::to_vec(&state)?, &directory)?;
  Ok(DomainRouteObservation {
    exists: state.fingerprint.is_some(),
    fingerprint: state.fingerprint,
    fencing_token: fence,
    load_verified: false,
    load_error: Some("IngressLoadFailed".into()),
    rolled_back: true,
  })
}

fn still_current(
  path: &Path,
  domain: &str,
  fence: u64,
  fingerprint: &Option<String>,
) -> Result<()> {
  let directory = lock_directory(path)?;
  let base =
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
  let mut state: State = serde_json::from_slice(
    &read(&base.join(format!(".komodo-{domain}.state")))?
      .context("UnownedRouteConflict")?,
  )?;
  ensure!(
    state.fence == fence
      && &state.fingerprint == fingerprint
      && &read(&base.join(format!("komodo-{domain}.yml")))?
        .as_ref()
        .map(|bytes| digest(bytes))
        == fingerprint,
    "ExternalDrift"
  );
  if let Some(bytes) =
    read(&base.join(format!("komodo-{domain}.yml")))?
  {
    atomic(
      &base.join(format!(".komodo-{domain}.last-valid")),
      &bytes,
      &directory,
    )?;
    state.last_valid_fingerprint = Some(digest(&bytes));
    atomic(
      &base.join(format!(".komodo-{domain}.state")),
      &serde_json::to_vec(&state)?,
      &directory,
    )?;
  }
  Ok(())
}

fn last_valid(path: &Path, domain: &str) -> Result<Option<Vec<u8>>> {
  let directory = lock_directory(path)?;
  let base =
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
  let bytes =
    read(&base.join(format!(".komodo-{domain}.last-valid")))?;
  let state: State =
    match read(&base.join(format!(".komodo-{domain}.state")))? {
      Some(bytes) => serde_json::from_slice(&bytes)?,
      None => State::default(),
    };
  ensure!(
    bytes.as_ref().map(|bytes| digest(bytes))
      == state.last_valid_fingerprint,
    "BackupDrift"
  );
  Ok(bytes)
}

fn remember_previous(
  path: &Path,
  domain: &str,
  bytes: &[u8],
) -> Result<()> {
  let directory = lock_directory(path)?;
  let base =
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
  let expected = digest(bytes);
  let mut state: State = serde_json::from_slice(
    &read(&base.join(format!(".komodo-{domain}.state")))?
      .context("UnownedRouteConflict")?,
  )?;
  ensure!(
    state.fingerprint.as_deref() == Some(expected.as_str())
      && read(&base.join(format!("komodo-{domain}.yml")))?
        .as_ref()
        .map(|bytes| digest(bytes))
        .as_deref()
        == Some(expected.as_str()),
    "ExternalDrift"
  );
  atomic(
    &base.join(format!(".komodo-{domain}.last-valid")),
    bytes,
    &directory,
  )?;
  state.last_valid_fingerprint = Some(expected);
  atomic(
    &base.join(format!(".komodo-{domain}.state")),
    &serde_json::to_vec(&state)?,
    &directory,
  )
}

async fn poll_loaded(
  url: &url::Url,
  domain: &str,
  bytes: &[u8],
) -> Loaded {
  let until =
    tokio::time::Instant::now() + std::time::Duration::from_secs(6);
  loop {
    match fetch_loaded(url, domain, bytes).await {
      Ok(Loaded::Verified) => return Loaded::Verified,
      Ok(Loaded::Failed) => return Loaded::Failed,
      _ => {}
    }
    if tokio::time::Instant::now() >= until {
      return Loaded::Pending;
    }
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
  }
}

impl Resolve<Args> for GetDomainIngressCapabilities {
  async fn resolve(
    self,
    _: &Args,
  ) -> Result<DomainIngressCapabilities> {
    let path = directory(&self.ingress_id)?;
    tokio::task::spawn_blocking(move || {
      let _guard = lock_directory(&path)?;
      Ok(DomainIngressCapabilities {
        rpc_version: 1,
        enabled: true,
        dedicated: true,
      })
    })
    .await?
  }
}
impl Resolve<Args> for ObserveDomainRoute {
  async fn resolve(self, _: &Args) -> Result<DomainRouteObservation> {
    let path = directory(&self.ingress_id)?;
    let endpoint = observation_url(&self.ingress_id)?;
    let domain = self.domain_id.clone();
    let observe_path = path.clone();
    let (mut result, snapshot) =
      tokio::task::spawn_blocking(move || -> Result<_> {
        let result = operate_before(
          &path,
          &domain,
          0,
          None,
          None,
          clock_ms()? + 60_000,
        )?;
        let snapshot = owned_snapshot(&path, &domain)?;
        Ok((result, snapshot))
      })
      .await??;
    if let (Some(endpoint), Some(bytes)) = (endpoint, snapshot) {
      ensure!(
        Some(digest(&bytes)) == result.fingerprint,
        "ExternalDrift"
      );
      match fetch_loaded(&endpoint, &self.domain_id, &bytes).await {
        Ok(Loaded::Verified) => {
          let fence = result.fencing_token;
          let fingerprint = result.fingerprint.clone();
          let domain = self.domain_id.clone();
          tokio::task::spawn_blocking(move || {
            still_current(&observe_path, &domain, fence, &fingerprint)
          })
          .await??;
          result.load_verified = true;
        }
        Ok(Loaded::Failed) => {
          result.load_error = Some("IngressLoadFailed".into())
        }
        _ => {}
      }
    }
    Ok(result)
  }
}
impl Resolve<Args> for ApplyDomainRoute {
  async fn resolve(self, _: &Args) -> Result<DomainRouteObservation> {
    let path = directory(&self.ingress_id)?;
    let endpoint = observation_url(&self.ingress_id)?;
    let domain = self.domain_id.clone();
    let before_path = path.clone();
    let before_domain = domain.clone();
    let snapshot_fence = self.fencing_token;
    let previous = tokio::task::spawn_blocking(move || {
      owned_snapshot_at_fence(
        &before_path,
        &before_domain,
        Some(snapshot_fence),
      )
    })
    .await??;
    let previous_verified = match (&endpoint, &previous) {
      (_, None) => true,
      (Some(url), Some(bytes)) => fetch_loaded(url, &domain, bytes)
        .await
        .is_ok_and(|state| state == Loaded::Verified),
      _ => false,
    };
    // Persist the observed-good snapshot before publishing, so retries after a
    // process restart can still compensate a failed configuration load.
    let backup_path = path.clone();
    let backup_domain = domain.clone();
    let previous_for_backup = previous.clone();
    let fallback = tokio::task::spawn_blocking(
      move || -> Result<Option<Vec<u8>>> {
        if previous_verified {
          if let Some(bytes) = &previous_for_backup {
            remember_previous(&backup_path, &backup_domain, bytes)?;
          }
          Ok(previous_for_backup)
        } else {
          last_valid(&backup_path, &backup_domain)
        }
      },
    )
    .await??;
    let can_rollback = previous_verified || fallback.is_some();
    let desired = render(&domain, &self.route)?;
    let fence = self.fencing_token;
    let deadline = self.expires_at_ms;
    let apply_path = path.clone();
    let apply_domain = domain.clone();
    let mut result = tokio::task::spawn_blocking(move || {
      operate_before(
        &apply_path,
        &apply_domain,
        fence,
        self.expected_fingerprint,
        Some(Mutation::Apply(self.route)),
        deadline,
      )
    })
    .await??;
    if let Some(endpoint) = endpoint {
      match poll_loaded(&endpoint, &domain, &desired).await {
        Loaded::Verified => {
          let verify_path = path.clone();
          let verify_domain = domain.clone();
          let fingerprint = result.fingerprint.clone();
          tokio::task::spawn_blocking(move || {
            still_current(
              &verify_path,
              &verify_domain,
              fence,
              &fingerprint,
            )
          })
          .await??;
          result.load_verified = true;
        }
        Loaded::Failed => {
          result.load_error = Some("IngressLoadFailed".into());
          if can_rollback
            && fallback.as_ref().map(|bytes| digest(bytes))
              != result.fingerprint
          {
            let failed = result
              .fingerprint
              .clone()
              .context("MissingPublishedFingerprint")?;
            result = tokio::task::spawn_blocking(move || {
              rollback(
                &path, &domain, fence, &failed, fallback, deadline,
              )
            })
            .await??;
          }
        }
        Loaded::Pending => {}
      }
    }
    Ok(result)
  }
}
impl Resolve<Args> for RemoveDomainRoute {
  async fn resolve(self, _: &Args) -> Result<DomainRouteObservation> {
    let path = directory(&self.ingress_id)?;
    tokio::task::spawn_blocking(move || {
      operate_before(
        &path,
        &self.domain_id,
        self.fencing_token,
        self.expected_fingerprint,
        Some(Mutation::Remove),
        self.expires_at_ms,
      )
    })
    .await?
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  struct Temp(PathBuf);
  impl Temp {
    fn new() -> Self {
      let path = std::env::temp_dir()
        .join(format!("komodo-domain-test-{}", uuid::Uuid::new_v4()));
      fs::create_dir(&path).unwrap();
      fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .unwrap();
      Self(path)
    }
  }
  impl Drop for Temp {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }
  fn route() -> DomainRoute {
    DomainRoute {
      hostname: "app.example.com".into(),
      backend_url: "http://app:8080".into(),
      entrypoints: vec!["websecure".into()],
      require_https: true,
      certificate_resolver: Some("le".into()),
    }
  }
  fn apply(
    path: &Path,
    token: u64,
    fingerprint: Option<String>,
  ) -> Result<DomainRouteObservation> {
    operate(
      path,
      "binding",
      token,
      fingerprint,
      Some(Mutation::Apply(route())),
    )
  }
  fn rawdata_fixture(config: &[u8]) -> serde_json::Value {
    let config: serde_json::Value =
      serde_json::from_slice(config).unwrap();
    let mut router =
      config["http"]["routers"]["komodo-binding"].clone();
    let mut service =
      config["http"]["services"]["komodo-binding"].clone();
    router["status"] = json!("enabled");
    service["status"] = json!("enabled");
    json!({"routers": {"komodo-binding@file": router}, "services": {"komodo-binding@file": service}})
  }
  #[test]
  fn rawdata_requires_exact_loaded_configuration() {
    let bytes = render("binding", &route()).unwrap();
    let mut raw = rawdata_fixture(&bytes);
    assert_eq!(
      verify_rawdata("binding", &bytes, &raw).unwrap(),
      Loaded::Verified
    );
    raw["routers"]["komodo-binding@file"]["rule"] =
      json!("Host(`other.example.com`)");
    assert_eq!(
      verify_rawdata("binding", &bytes, &raw).unwrap(),
      Loaded::Pending
    );
    raw = rawdata_fixture(&bytes);
    raw["services"]["komodo-binding@file"]["loadBalancer"]["servers"]
      [0]["url"] = json!("http://other:9090");
    assert_eq!(
      verify_rawdata("binding", &bytes, &raw).unwrap(),
      Loaded::Pending
    );
    raw = rawdata_fixture(&bytes);
    raw["routers"]["komodo-binding@file"]["status"] =
      json!("disabled");
    assert_eq!(
      verify_rawdata("binding", &bytes, &raw).unwrap(),
      Loaded::Failed
    );
    assert_eq!(
      verify_rawdata("binding", &bytes, &json!({})).unwrap(),
      Loaded::Pending
    );
  }
  #[tokio::test]
  async fn http_observation_reads_rawdata_without_following_redirects()
   {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn mock(status: &str, body: String) -> url::Url {
      let listener =
        tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
      let address = listener.local_addr().unwrap();
      let status = status.to_owned();
      tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        let count = socket.read(&mut request).await.unwrap();
        assert!(
          String::from_utf8_lossy(&request[..count])
            .starts_with("GET /api/rawdata ")
        );
        let response = format!(
          "HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/api/rawdata\r\nConnection: close\r\n\r\n{body}",
          body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
      });
      url::Url::parse(&format!("http://{address}/api/rawdata"))
        .unwrap()
    }
    let bytes = render("binding", &route()).unwrap();
    let endpoint =
      mock("200 OK", rawdata_fixture(&bytes).to_string()).await;
    assert_eq!(
      fetch_loaded(&endpoint, "binding", &bytes).await.unwrap(),
      Loaded::Verified
    );
    let endpoint = mock("302 Found", String::new()).await;
    assert!(
      fetch_loaded(&endpoint, "binding", &bytes).await.is_err()
    );
    let endpoint = mock("200 OK", "not json".into()).await;
    assert!(
      fetch_loaded(&endpoint, "binding", &bytes).await.is_err()
    );
  }
  #[test]
  fn rollback_requires_current_fence_and_unchanged_file() {
    let temp = Temp::new();
    let original = apply(&temp.0, 1, None).unwrap();
    let old_bytes = owned_snapshot(&temp.0, "binding").unwrap();
    remember_previous(
      &temp.0,
      "binding",
      old_bytes.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(last_valid(&temp.0, "binding").unwrap(), old_bytes);
    let mut changed = route();
    changed.backend_url = "http://other:9090".into();
    let updated = operate(
      &temp.0,
      "binding",
      2,
      original.fingerprint.clone(),
      Some(Mutation::Apply(changed)),
    )
    .unwrap();
    assert!(
      rollback(
        &temp.0,
        "binding",
        1,
        updated.fingerprint.as_deref().unwrap(),
        old_bytes.clone(),
        clock_ms().unwrap() + 60_000
      )
      .is_err()
    );
    let restored = rollback(
      &temp.0,
      "binding",
      2,
      updated.fingerprint.as_deref().unwrap(),
      old_bytes,
      clock_ms().unwrap() + 60_000,
    )
    .unwrap();
    assert!(restored.rolled_back && restored.load_error.is_some());
    assert_eq!(restored.fingerprint, original.fingerprint);
    fs::write(
      temp.0.join(".komodo-binding.last-valid"),
      "tampered backup",
    )
    .unwrap();
    assert!(last_valid(&temp.0, "binding").is_err());
    fs::write(temp.0.join("komodo-binding.yml"), "manual edit")
      .unwrap();
    assert!(
      rollback(
        &temp.0,
        "binding",
        2,
        restored.fingerprint.as_deref().unwrap(),
        None,
        clock_ms().unwrap() + 60_000
      )
      .is_err()
    );
  }
  #[test]
  fn expired_or_unbounded_lease_cannot_write() {
    let temp = Temp::new();
    let now = clock_ms().unwrap();
    assert!(
      operate_before(
        &temp.0,
        "binding",
        1,
        None,
        Some(Mutation::Apply(route())),
        now - 1
      )
      .is_err()
    );
    assert!(
      operate_before(
        &temp.0,
        "binding",
        1,
        None,
        Some(Mutation::Apply(route())),
        now + 120_000
      )
      .is_err()
    );
    assert!(!temp.0.join("komodo-binding.yml").exists());
  }
  #[test]
  fn publish_replay_fencing_drift_and_cleanup() {
    let temp = Temp::new();
    let first = apply(&temp.0, 10, None).unwrap();
    assert!(first.exists && !first.load_verified);
    assert_eq!(
      apply(&temp.0, 10, None).unwrap().fingerprint,
      first.fingerprint
    );
    assert!(apply(&temp.0, 9, first.fingerprint.clone()).is_err());
    let original =
      fs::read(temp.0.join("komodo-binding.yml")).unwrap();
    fs::write(temp.0.join("komodo-binding.yml"), "manual edit")
      .unwrap();
    assert!(apply(&temp.0, 11, first.fingerprint.clone()).is_err());
    assert!(
      operate(
        &temp.0,
        "binding",
        11,
        Some(digest(b"manual edit")),
        Some(Mutation::Remove)
      )
      .is_err()
    );
    fs::write(temp.0.join("komodo-binding.yml"), original).unwrap();
    assert!(
      !operate(
        &temp.0,
        "binding",
        12,
        first.fingerprint,
        Some(Mutation::Remove)
      )
      .unwrap()
      .exists
    );
    assert!(
      !operate(&temp.0, "binding", 12, None, Some(Mutation::Remove))
        .unwrap()
        .exists
    );
  }
  #[test]
  fn never_adopt_by_filename_and_reject_paths_and_symlinks() {
    let temp = Temp::new();
    fs::write(temp.0.join("komodo-binding.yml"), "manual").unwrap();
    assert!(apply(&temp.0, 1, Some(digest(b"manual"))).is_err());
    assert!(operate(&temp.0, "../escape", 1, None, None).is_err());
    fs::remove_file(temp.0.join("komodo-binding.yml")).unwrap();
    std::os::unix::fs::symlink(
      "/etc/passwd",
      temp.0.join("komodo-binding.yml"),
    )
    .unwrap();
    assert!(apply(&temp.0, 1, None).is_err());
  }
  #[test]
  fn refuse_foreign_configuration_and_duplicate_hostnames() {
    let temp = Temp::new();
    fs::write(temp.0.join("manual.yml"), "http: {}").unwrap();
    assert!(apply(&temp.0, 1, None).is_err());
    fs::remove_file(temp.0.join("manual.yml")).unwrap();
    let first = apply(&temp.0, 1, None).unwrap();
    assert!(
      operate(
        &temp.0,
        "other",
        2,
        None,
        Some(Mutation::Apply(route()))
      )
      .is_err()
    );
    let mut changed = route();
    changed.backend_url = "http://new:9090".into();
    assert!(
      operate(
        &temp.0,
        "binding",
        1,
        first.fingerprint.clone(),
        Some(Mutation::Apply(changed))
      )
      .is_err()
    );
    assert!(
      operate(
        &temp.0,
        "binding",
        1,
        first.fingerprint,
        Some(Mutation::Remove)
      )
      .is_err()
    );
  }
  #[test]
  fn invalid_input_preserves_last_configuration() {
    let temp = Temp::new();
    let first = apply(&temp.0, 1, None).unwrap();
    let mut invalid = route();
    invalid.hostname =
      "app.example.com`) || Host(`other.example.com".into();
    assert!(
      operate(
        &temp.0,
        "binding",
        2,
        first.fingerprint.clone(),
        Some(Mutation::Apply(invalid))
      )
      .is_err()
    );
    assert_eq!(
      operate(&temp.0, "binding", 0, None, None)
        .unwrap()
        .fingerprint,
      first.fingerprint
    );
    let mut invalid = route();
    invalid.backend_url = "http://user:secret@app".into();
    assert!(render("binding", &invalid).is_err());
  }
  #[test]
  fn recover_published_and_loaded_file_before_snapshot_backup() {
    let temp = Temp::new();
    let bytes = render("binding", &route()).unwrap();
    let desired = digest(&bytes);
    let state_path = temp.0.join(".komodo-binding.state");
    let route_path = temp.0.join("komodo-binding.yml");
    let pending = State {
      fence: 8,
      fingerprint: None,
      pending: true,
      pending_fingerprint: Some(desired.clone()),
      last_valid_fingerprint: None,
    };
    fs::write(&state_path, serde_json::to_vec(&pending).unwrap())
      .unwrap();
    fs::write(&route_path, &bytes).unwrap();
    // The old lease must not settle or snapshot a newer task's durable intent.
    assert!(
      owned_snapshot_at_fence(&temp.0, "binding", Some(7)).is_err()
    );
    fs::write(&route_path, b"manual drift").unwrap();
    assert!(
      owned_snapshot_at_fence(&temp.0, "binding", Some(8)).is_err()
    );
    let unsettled: State =
      serde_json::from_slice(&fs::read(&state_path).unwrap())
        .unwrap();
    assert!(unsettled.pending && unsettled.fingerprint.is_none());
    fs::write(&route_path, &bytes).unwrap();
    // Simulate retry after rename and successful Traefik watch, before the
    // original process could commit its final state (the uncovered crash point).
    let snapshot =
      owned_snapshot_at_fence(&temp.0, "binding", Some(8))
        .unwrap()
        .unwrap();
    assert_eq!(
      verify_rawdata("binding", &snapshot, &rawdata_fixture(&bytes))
        .unwrap(),
      Loaded::Verified
    );
    remember_previous(&temp.0, "binding", &snapshot).unwrap();
    assert_eq!(last_valid(&temp.0, "binding").unwrap(), Some(bytes));
    assert_eq!(
      apply(&temp.0, 8, None).unwrap().fingerprint,
      Some(desired)
    );
    let settled: State =
      serde_json::from_slice(&fs::read(&state_path).unwrap())
        .unwrap();
    assert!(!settled.pending && settled.fence == 8);
  }
  #[test]
  fn recover_interrupted_publication_and_deletion() {
    let temp = Temp::new();
    let desired = digest(&render("binding", &route()).unwrap());
    let state_path = temp.0.join(".komodo-binding.state");
    let pending = State {
      fence: 4,
      fingerprint: None,
      pending: true,
      pending_fingerprint: Some(desired.clone()),
      last_valid_fingerprint: None,
    };
    fs::write(&state_path, serde_json::to_vec(&pending).unwrap())
      .unwrap();
    // Crash before rename: retry publishes the same intent.
    assert_eq!(
      apply(&temp.0, 4, None).unwrap().fingerprint,
      Some(desired.clone())
    );
    let pending = State {
      fence: 5,
      fingerprint: Some(desired),
      pending: true,
      pending_fingerprint: None,
      last_valid_fingerprint: None,
    };
    fs::write(&state_path, serde_json::to_vec(&pending).unwrap())
      .unwrap();
    fs::remove_file(temp.0.join("komodo-binding.yml")).unwrap();
    // Crash after unlink: retry completes tombstone persistence.
    assert!(
      !operate(&temp.0, "binding", 5, None, Some(Mutation::Remove))
        .unwrap()
        .exists
    );
  }
}
