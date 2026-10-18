pub mod adapters;
pub mod lease;
mod observation;
pub mod policy;
mod reconcile;
mod target;
pub mod validation;
use crate::{resource, state::db_client};
use adapters::*;
use anyhow::{Context, bail};
use database::bson::{Document, doc, oid::ObjectId, to_bson};
use komodo_client::entities::{
  domain::*, komodo_timestamp, permission::PermissionLevel,
  user::User,
};
use serde::{Deserialize, Serialize};

pub fn dns_digest(value: &serde_json::Value) -> String {
  let mut values = value
    .as_array()
    .map(|a| a.iter().map(dns_fingerprint).collect::<Vec<_>>())
    .unwrap_or_default();
  values.sort();
  fingerprint(&serde_json::json!(values))
}
pub fn digest(value: &impl Serialize) -> anyhow::Result<String> {
  Ok(fingerprint(&serde_json::to_value(value)?))
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedPlan {
  #[serde(default)]
  pub display_hostname: String,
  pub public: DomainPlan,
  pub config: DomainConfig,
  pub author: String,
  pub ingress: Ingress,
  pub provider: Option<DnsProvider>,
  pub backend: String,
  pub dns_observed: serde_json::Value,
  pub ingress_observed: serde_json::Value,
  pub objects: Vec<String>,
  pub instance: String,
}
pub async fn load_plan(
  id: &str,
  user: &User,
) -> anyhow::Result<SavedPlan> {
  validation::enabled()?;
  let doc = db_client()
    .domain_plans
    .find_one(doc! {"_id":id})
    .await?
    .context("NotFound: domain plan")?;
  let plan: SavedPlan = database::bson::from_document(doc)?;
  if plan.author != user.id && !user.admin {
    bail!("Forbidden: plan belongs to another caller")
  }
  if let Some(id) = &plan.public.domain_id
    && plan.public.operation != DomainPlanOperation::Create
  {
    authorized::<Domain>(id, user, PermissionLevel::Read.into())
      .await?;
  }
  Ok(plan)
}
pub async fn plan(
  intent: DomainChangeIntent,
  user: &User,
) -> anyhow::Result<DomainPlan> {
  validation::enabled()?;
  let (domain, mut config, operation, objects) = match intent {
    DomainChangeIntent::Create { name, config } => {
      let name = komodo_client::entities::to_general_name(&name);
      if name.trim().is_empty() {
        bail!("InvalidConfig: Domain name must not be empty")
      }
      if !user.admin
        && crate::config::core_config().disable_non_admin_create
      {
        bail!("Forbidden: domain creation disabled")
      }
      if db_client()
        .domains
        .count_documents(doc! {"name":&name})
        .await?
        > 0
      {
        bail!("Conflict: domain name already exists")
      }
      (
        Domain {
          name,
          ..Default::default()
        },
        config,
        DomainPlanOperation::Create,
        vec![],
      )
    }
    DomainChangeIntent::Update {
      domain,
      config,
      expected_generation,
    } => {
      let old = authorized::<Domain>(
        &domain,
        user,
        PermissionLevel::Write.into(),
      )
      .await?;
      if old.info.generation != expected_generation {
        bail!("GenerationConflict: replan current configuration")
      }
      (old, config, DomainPlanOperation::Update, vec![])
    }
    other => {
      let (id, op, provider, ingress, objects) = match other {
        DomainChangeIntent::Reconcile { domain } => {
          (domain, DomainPlanOperation::Reconcile, None, None, vec![])
        }
        DomainChangeIntent::Remove { domain } => {
          (domain, DomainPlanOperation::Remove, None, None, vec![])
        }
        DomainChangeIntent::Migrate {
          domain,
          dns_provider,
          ingress,
        } => (
          domain,
          DomainPlanOperation::Migrate,
          dns_provider,
          ingress,
          vec![],
        ),
        DomainChangeIntent::Adopt { domain, objects } => {
          (domain, DomainPlanOperation::Adopt, None, None, objects)
        }
        DomainChangeIntent::Abandon { domain } => {
          (domain, DomainPlanOperation::Abandon, None, None, vec![])
        }
        DomainChangeIntent::FinalizeMigration { domain } => (
          domain,
          DomainPlanOperation::FinalizeMigration,
          None,
          None,
          vec![],
        ),
        _ => unreachable!(),
      };
      let level = if op == DomainPlanOperation::Reconcile {
        PermissionLevel::Execute
      } else {
        PermissionLevel::Write
      };
      let old = authorized::<Domain>(&id, user, level.into()).await?;
      let mut config = old.config.clone();
      if op == DomainPlanOperation::Migrate
        && provider.is_none()
        && ingress.is_none()
      {
        bail!("InvalidConfig: provide a replacement connection")
      }
      if let Some(provider) = provider {
        match &mut config.dns {
          DomainDns::Managed { provider: p, .. } => *p = provider,
          _ => bail!(
            "UnsupportedCapability: migrate from External DNS requires an explicit managed configuration"
          ),
        }
      }
      if let Some(ingress) = ingress {
        config.ingress = ingress;
      }
      (old, config, op, objects)
    }
  };
  let display_hostname =
    if matches!(
      operation,
      DomainPlanOperation::Create | DomainPlanOperation::Update
    ) || domain.info.display_hostname.is_empty()
    {
      config.hostname.clone()
    } else {
      domain.info.display_hostname.clone()
    };
  let (ingress, provider, backend) =
    validation::resolve_for_operation(&mut config, user, &operation)
      .await?;
  if operation == DomainPlanOperation::Create
    && db_client()
      .domains
      .count_documents(doc! {"config.hostname":&config.hostname})
      .await?
      > 0
  {
    bail!(
      "HostnameConflict: normalized hostname is already bound to another Domain"
    )
  }
  if operation == DomainPlanOperation::Update
    && (config.ingress != domain.config.ingress
      || provider_ref(&config.dns)
        != provider_ref(&domain.config.dns)
      || config.hostname != domain.config.hostname)
  {
    bail!(
      "MigrationRequired: connection or hostname cannot be overwritten"
    )
  }
  let id = if domain.id.is_empty() {
    ObjectId::new().to_hex()
  } else {
    domain.id.clone()
  };
  let dns_observed = if operation == DomainPlanOperation::Abandon {
    serde_json::Value::Null
  } else {
    observe_dns(provider.as_ref(), &config.hostname).await?
  };
  let ingress_observed = if operation == DomainPlanOperation::Abandon
  {
    serde_json::Value::Null
  } else {
    observe_ingress(&ingress, &id).await?
  };
  reconcile::check_plan_objects(
    &id,
    &operation,
    &dns_observed,
    &ingress,
    &ingress_observed,
    &config.hostname,
    &objects,
  )
  .await?;
  let mut warnings = vec![];
  if matches!(config.dns, DomainDns::External) {
    warnings.push("DNS is externally managed; change and verify resolution manually".into())
  }
  let requires_downtime = operation == DomainPlanOperation::Migrate
    && (matches!(
      ingress.config.adapter,
      IngressAdapterConfig::CloudflareTunnelV1 { .. }
    ) || ingress
      .config
      .tls_profiles
      .iter()
      .any(|p| p.name == config.tls.profile && p.requires_dns));
  if requires_downtime {
    warnings.push("Certificate bootstrap requires DNS cutover; downtime may occur".into())
  }
  if operation == DomainPlanOperation::Abandon {
    warnings.push(
      "External objects are retained and ownership is relinquished"
        .into(),
    )
  }
  if operation == DomainPlanOperation::Adopt && objects.len() != 1 {
    bail!(
      "InvalidConfig: exactly one precise DNS record or Tunnel hostname must be adopted per plan"
    )
  }
  if operation == DomainPlanOperation::Adopt {
    let object = &objects[0];
    let dns_match = dns_observed.as_array().is_some_and(|a| {
      a.iter().any(|r| r["id"].as_str() == Some(object))
    });
    let tunnel_match = matches!(
      ingress.config.adapter,
      IngressAdapterConfig::CloudflareTunnelV1 { .. }
    ) && object == &config.hostname
      && ingress_observed["ingress"].as_array().is_some_and(|a| {
        a.iter().any(|r| r["hostname"] == config.hostname)
      });
    if !dns_match && !tunnel_match {
      bail!(
        "UnsupportedCapability: adopt requires an observed DNS record ID or exact Tunnel hostname; manual Traefik file adoption is not supported"
      )
    }
    warnings.push(format!("Explicitly acquire deletion rights to object {object}; existing content is compared before mutation"));
  }
  let steps = review_steps(
    &operation,
    &id,
    &config,
    &ingress,
    &backend,
    &dns_observed,
    &objects,
  )
  .await?;
  let public = DomainPlan {
    plan_id: uuid::Uuid::new_v4().to_string(),
    domain_id: Some(id),
    name: domain.name,
    generation: domain.info.generation,
    expires_at: komodo_timestamp() + 900_000,
    operation,
    steps,
    warnings,
    requires_downtime,
    request_id: uuid::Uuid::new_v4().to_string(),
  };
  let saved = SavedPlan {
    display_hostname,
    public: public.clone(),
    config,
    author: user.id.clone(),
    ingress,
    provider,
    backend,
    dns_observed,
    ingress_observed,
    objects,
    instance: crate::config::core_config().domain_instance_id.clone(),
  };
  let mut doc = database::bson::to_document(&saved)?;
  doc.insert("_id", &public.plan_id);
  db_client().domain_plans.insert_one(doc).await?;
  Ok(public)
}
fn provider_ref(dns: &DomainDns) -> Option<&str> {
  match dns {
    DomainDns::Managed { provider, .. } => Some(provider),
    _ => None,
  }
}
async fn observe_dns(
  provider: Option<&DnsProvider>,
  hostname: &str,
) -> anyhow::Result<serde_json::Value> {
  if let Some(provider) = provider {
    let driver = DnsDriver::from_provider(provider)?;
    let records = driver.observe(hostname).await?;
    Ok(serde_json::to_value(records)?)
  } else {
    Ok(serde_json::Value::Null)
  }
}
async fn periphery(
  ingress: &Ingress,
) -> anyhow::Result<crate::periphery::PeripheryClient> {
  if let IngressAdapterConfig::TraefikFileV1 { server, .. } =
    &ingress.config.adapter
  {
    let server = resource::get::<
      komodo_client::entities::server::Server,
    >(server)
    .await?;
    crate::helpers::periphery_client(&server).await
  } else {
    bail!("UnsupportedCapability: not Traefik")
  }
}
async fn observe_ingress(
  ingress: &Ingress,
  id: &str,
) -> anyhow::Result<serde_json::Value> {
  Ok(
    IngressDriver::from_ingress(ingress)
      .await?
      .observe(id)
      .await?,
  )
}
pub fn capabilities(
  adapter: &str,
) -> anyhow::Result<DomainAdapterCapabilities> {
  Ok(adapters::registry::capabilities(adapter)?)
}
pub async fn validate_connection(
  connection: DomainConnection,
  user: &User,
) -> anyhow::Result<DomainConnectionValidation> {
  validation::enabled()?;
  let capabilities = match connection {
    DomainConnection::DnsProvider(id) => {
      let provider = authorized::<DnsProvider>(
        &id,
        user,
        PermissionLevel::Write.into(),
      )
      .await?;
      let driver = DnsDriver::from_provider(&provider)?;
      driver.validate().await?;
      driver.capabilities()
    }
    DomainConnection::Ingress(id) => {
      let ingress = authorized::<Ingress>(
        &id,
        user,
        PermissionLevel::Write.into(),
      )
      .await?;
      let driver = IngressDriver::from_ingress(&ingress).await?;
      driver.validate().await?;
      driver.capabilities()
    }
  };
  Ok(DomainConnectionValidation {
    valid: true,
    capabilities,
    errors: vec![],
  })
}
pub use reconcile::{accept, recover_interrupted, start_automatic};

pub async fn refresh_status(
  domain: &mut Domain,
) -> anyhow::Result<()> {
  if domain.info.observed_generation != domain.info.generation {
    return Ok(());
  }
  if !matches!(
    domain.info.state,
    DomainState::Degraded | DomainState::Ready
  ) {
    return Ok(());
  }
  let ingress =
    resource::get::<Ingress>(&domain.config.ingress).await?;
  let mut conditions = observation::observe_readiness(
    &domain.config,
    &ingress,
    &domain.id,
  )
  .await;
  conditions.push(observe_target(domain, &ingress).await);
  domain.info.state =
    if conditions.iter().all(|c| c.status == ConditionStatus::True) {
      DomainState::Ready
    } else {
      DomainState::Degraded
    };
  domain.info.conditions = conditions;
  db_client().domains.update_one(doc!{"_id":ObjectId::parse_str(&domain.id)?,"info.generation":domain.info.generation,"info.state":{"$in":["Ready","Degraded"]}},doc!{"$set":{"info.state":to_bson(&domain.info.state)?,"info.conditions":to_bson(&domain.info.conditions)?}}).await?;
  Ok(())
}

pub async fn authorized<T: crate::resource::KomodoResource>(
  id: &str,
  user: &User,
  permissions:komodo_client::entities::permission::PermissionLevelAndSpecifics,
) -> anyhow::Result<
  komodo_client::entities::resource::Resource<T::Config, T::Info>,
> {
  crate::permission::get_check_permissions::<T>(id, user, permissions)
    .await
    .map_err(|error| {
      let message = error.to_string();
      if message.contains("required permissions") {
        anyhow::anyhow!(
          "Forbidden: required resource permissions are missing"
        )
      } else if message.contains("Did not find")
        || message.contains("Cannot find")
      {
        anyhow::anyhow!(
          "NotFound: referenced resource does not exist"
        )
      } else {
        error
      }
    })
}

async fn review_steps(
  operation: &DomainPlanOperation,
  id: &str,
  config: &DomainConfig,
  ingress: &Ingress,
  backend: &str,
  dns: &serde_json::Value,
  objects: &[String],
) -> anyhow::Result<Vec<String>> {
  let ownership = db_client()
    .domain_ownership
    .find_one(doc! {"_id":id})
    .await?;
  let describe = |state: Option<&Document>| -> String {
    let Some(state) = state else {
      return "none".into();
    };
    ["dns", "ingress"]
      .iter()
      .filter_map(|kind| {
        state.get_document(kind).ok().map(|handle| {
          format!(
            "{kind} object {} (fingerprint {})",
            handle.get_str("id").unwrap_or("unknown"),
            handle.get_str("fingerprint").unwrap_or("unknown")
          )
        })
      })
      .collect::<Vec<_>>()
      .join(", ")
  };
  let active = describe(ownership.as_ref());
  let old = describe(
    ownership.as_ref().and_then(|s| s.get_document("old").ok()),
  );
  let steps=match operation {
  DomainPlanOperation::Remove=>vec![format!("Withdraw only owned DNS for {}: {active}",config.hostname),format!("Drain for {} seconds before removing owned ingress routes",ingress.config.drain_seconds),format!("Clean any retained migration objects: {old}"),"Keep the Domain tombstone until cleanup succeeds; never remove external/manual DNS".into()],
  DomainPlanOperation::Abandon=>vec![format!("Record an audit inventory of retained external objects: {active}; previous migration: {old}"),"Relinquish management and deletion rights; leave all DNS records and routes unchanged".into()],
  DomainPlanOperation::FinalizeMigration=>vec!["Verify current DNS, active ingress, TLS and public HTTP health, then confirm drain deadline".into(),format!("Remove only retained old migration objects: {old}"),format!("Keep active objects: {active}")],
  DomainPlanOperation::Adopt=>vec![format!("Explicitly acquire ownership of exactly these observed objects: {}",objects.join(", ")),format!("After exact fingerprint comparison, converge {} to backend {} using ingress {}",config.hostname,backend,ingress.name),"Preserve all objects not explicitly listed for adoption".into()],
  operation=>{
   let mut steps=vec![format!("{:?}: save canonical binding {} (config digest {})",operation,config.hostname,digest(config)?),format!("Prepare route on ingress {} to backend {} with TLS {:?}/{}",ingress.name,backend,config.tls.mode,config.tls.profile)];
   if *operation==DomainPlanOperation::Migrate {steps.push(format!("Retain active objects during candidate preparation: {active}; require explicit finalize before cleanup"));}
   if let DomainDns::Managed{ttl,proxy_mode,..}=&config.dns{
    let target=IngressDriver::from_ingress(ingress).await?.dns_target();
    let observed=dns.as_array().map(|records|records.iter().map(|r|format!("{} {} {}",r["id"].as_str().unwrap_or("?"),r["type"].as_str().unwrap_or("?"),r["content"].as_str().unwrap_or("?"))).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    steps.push(format!("Observed DNS: [{}]; converge owned address record to {}, TTL {}, proxy {:?}; refuse foreign records",observed,target,ttl,proxy_mode));
   }else{steps.push("DNS remains externally managed; wait for manual DNS changes and observed resolution".into());}
   steps.push("Observe DNS, loaded ingress, TLS and HTTP separately; unavailable probes remain Unknown".into());steps
  }
 };
  Ok(steps)
}

pub async fn check_tunnel_baseline(
  ingress: &Ingress,
  current: &serde_json::Value,
  pending: Option<&str>,
  _explicit_adopt: bool,
) -> anyhow::Result<()> {
  let IngressAdapterConfig::CloudflareTunnelV1 {
    account_id,
    tunnel_id,
    ..
  } = &ingress.config.adapter
  else {
    return Ok(());
  };
  let key = format!("tunnel:{account_id}:{tunnel_id}");
  let baseline = db_client()
    .db
    .collection::<Document>("DomainIngressState")
    .find_one(doc! {"_id":&key})
    .await?;
  if let Some(baseline) = baseline {
    if baseline.get_str("instance")
      != Ok(crate::config::core_config().domain_instance_id.as_str())
    {
      bail!(
        "OwnershipUnconfirmed: shared Tunnel belongs to another controller instance"
      )
    }
    let current = fingerprint(current);
    if baseline.get_str("fingerprint") != Ok(current.as_str())
      && pending != Some(current.as_str())
    {
      bail!(
        "Drifted: shared Tunnel changed outside Komodo; restore the last observed shared configuration before planning another change"
      )
    }
  }
  Ok(())
}
pub async fn save_tunnel_baseline(
  ingress: &Ingress,
  current: &serde_json::Value,
) -> anyhow::Result<()> {
  let IngressAdapterConfig::CloudflareTunnelV1 {
    account_id,
    tunnel_id,
    ..
  } = &ingress.config.adapter
  else {
    return Ok(());
  };
  let key = format!("tunnel:{account_id}:{tunnel_id}");
  db_client().db.collection::<Document>("DomainIngressState").replace_one(doc!{"_id":&key},doc!{"_id":&key,"instance":&crate::config::core_config().domain_instance_id,"fingerprint":fingerprint(current)}).upsert(true).await?;
  Ok(())
}

async fn observe_target(
  domain: &Domain,
  ingress: &Ingress,
) -> DomainCondition {
  let observe = async {
    let backend = match &domain.config.target {
      DomainTarget::Deployment { id, port, scheme } => {
        let deployment = resource::get::<
          komodo_client::entities::deployment::Deployment,
        >(id)
        .await?;
        target::deployment(&deployment, *port, scheme, ingress)
          .await?
      }
      DomainTarget::StackService {
        stack_id,
        service,
        port,
        scheme,
      } => {
        let stack = resource::get::<
          komodo_client::entities::stack::Stack,
        >(stack_id)
        .await?;
        target::stack_service(&stack, service, *port, scheme, ingress)
          .await?
      }
      DomainTarget::Url { url } => validation::validate_url(
        url,
        &ingress.config.allowed_networks,
      )?,
    };
    let ownership = db_client()
      .domain_ownership
      .find_one(doc! {"_id":&domain.id})
      .await?
      .context("TargetUnresolved: ownership missing")?;
    if ownership.get_str("resolved_backend") != Ok(backend.as_str()) {
      bail!(
        "TargetUnresolved: target backend changed since apply; reconcile the new backend"
      )
    }
    anyhow::Ok(())
  };
  let (status, reason, message) = match tokio::time::timeout(
    std::time::Duration::from_secs(12),
    observe,
  )
  .await
  {
    Ok(Ok(())) => (
      ConditionStatus::True,
      "Resolved",
      if matches!(domain.config.target, DomainTarget::Url { .. }) {
        "Explicit URL and administrator origin scope match the applied route; HTTP health is observed separately"
      } else {
        "Running target and ingress Docker network were verified against the applied backend"
      },
    ),
    Ok(Err(_)) => (
      ConditionStatus::False,
      "TargetUnresolved",
      "Target is stopped, changed, outside the network, or could not be inspected; reconcile after resolving the target",
    ),
    Err(_) => (
      ConditionStatus::Unknown,
      "ObservationTimeout",
      "Target network inspection did not complete within its observation deadline",
    ),
  };
  DomainCondition {
    condition: DomainConditionType::TargetResolved,
    status,
    reason: reason.into(),
    message: message.into(),
    observed_at: komodo_timestamp(),
  }
}
