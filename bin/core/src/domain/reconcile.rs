use super::*;
use crate::helpers::update::{
  add_update, make_update, update_update,
};
use komodo_client::entities::{
  Operation, ResourceTarget, update::Update,
};
use lease::Lease;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Ownership {
  #[serde(default)]
  instance: String,
  #[serde(default)]
  operator: String,
  #[serde(default)]
  resolved_backend: String,
  #[serde(default)]
  dns: Option<OwnedObject>,
  #[serde(default)]
  ingress: Option<OwnedObject>,
  #[serde(default)]
  pending_tunnel: Option<(OwnedObject, String)>,
  #[serde(default)]
  pending_dns: Option<String>,
  #[serde(default)]
  config: Option<DomainConfig>,
  #[serde(default)]
  old: Option<Box<Ownership>>,
  #[serde(default)]
  references: Vec<String>,
  #[serde(default)]
  cleaned: bool,
  #[serde(default)]
  cutover_at: i64,
  #[serde(default)]
  migration_plan: String,
}
async fn state(id: &str) -> anyhow::Result<Ownership> {
  Ok(
    db_client()
      .db
      .collection::<Document>("DomainOwnership")
      .find_one(doc! {"_id":id})
      .await?
      .map(database::bson::from_document)
      .transpose()?
      .unwrap_or_default(),
  )
}
async fn save(id: &str, state: &Ownership) -> anyhow::Result<()> {
  let mut doc = database::bson::to_document(state)?;
  doc.insert("_id", id);
  db_client()
    .db
    .collection::<Document>("DomainOwnership")
    .replace_one(doc! {"_id":id}, doc)
    .upsert(true)
    .await?;
  Ok(())
}
fn level(op: &DomainPlanOperation) -> PermissionLevel {
  super::policy::required_permission(op)
}

/// Admission is synchronous. No Accepted result is returned before permissions,
/// immutable request identity, plan freshness and generation checks pass.
pub async fn accept(
  domain: String,
  plan_id: String,
  expected: i64,
  request_id: String,
  kind: &str,
  user: User,
) -> anyhow::Result<Update> {
  validation::enabled()?;
  let public = if plan_id.is_empty() {
    let intent = match kind {
      "ReconcileDomain" => DomainChangeIntent::Reconcile {
        domain: domain.clone(),
      },
      "FinalizeDomainMigration" => {
        DomainChangeIntent::FinalizeMigration {
          domain: domain.clone(),
        }
      }
      _ => bail!("PlanRequired: explicit server plan is required"),
    };
    plan(intent, &user).await?
  } else {
    load_plan(&plan_id, &user).await?.public
  };
  let plan = load_plan(&public.plan_id, &user).await?;
  let id = public
    .domain_id
    .as_ref()
    .context("InvalidPlan: missing reserved domain ID")?
    .clone();
  if !domain.is_empty() && domain != id && domain != public.name {
    bail!("InvalidPlan: domain does not match plan")
  }
  let expected = if plan_id.is_empty() {
    public.generation
  } else {
    expected
  };
  if public.generation != expected {
    bail!(
      "GenerationConflict: plan generation does not match request"
    )
  }
  let request_id = if request_id.is_empty() {
    public.request_id.clone()
  } else {
    request_id
  };
  if request_id.len() > 128 || request_id.is_empty() {
    bail!("InvalidRequestId: request ID length must be 1..128")
  }
  let request_digest = super::policy::request_digest(
    &public.plan_id,
    expected,
    kind,
    &user.id,
  )?;
  super::policy::validate_plan_instance(
    &crate::config::core_config().domain_instance_id,
    &plan.instance,
  )?;
  let mut authorized = plan.config.clone();
  let (current_ingress, current_provider, current_backend) =
    validation::resolve_for_operation(
      &mut authorized,
      &user,
      &public.operation,
    )
    .await?;
  if current_backend != plan.backend
    || current_ingress.info.generation != plan.ingress.info.generation
    || current_provider.as_ref().map(|p| p.info.generation)
      != plan.provider.as_ref().map(|p| p.info.generation)
  {
    bail!("StalePlan: connection or target changed since planning")
  }
  if public.operation != DomainPlanOperation::Create {
    crate::domain::authorized::<Domain>(
      &id,
      &user,
      level(&public.operation).into(),
    )
    .await?;
  }
  let existing = db_client()
    .domain_operations
    .find_one(doc! {"request_id":&request_id})
    .await?;
  let recovering = existing.is_some();
  if let Some(op) = &existing {
    super::policy::validate_request_digest(
      op.get_str("digest")?,
      &request_digest,
    )?;
    if let Ok(update_id) = op.get_str("update_id") {
      let mut update = database::mungos::by_id::find_one_by_id(
        &db_client().updates,
        update_id,
      )
      .await?
      .context("OperationRecoveryRequired: update not found")?;
      if matches!(
        op.get_str("state"),
        Ok("Interrupted" | "Retryable")
      ) {
        let current = resource::get::<Domain>(&id).await?;
        if current.config != plan.config {
          bail!(
            "StalePlan: desired state changed after interrupted operation"
          )
        }
        if public.operation != DomainPlanOperation::Create {
          crate::domain::authorized::<Domain>(
            &id,
            &user,
            level(&public.operation).into(),
          )
          .await?;
        }
        let lease = Lease::acquire(&format!("domain:{id}")).await?;
        let current = resource::get::<Domain>(&id).await?;
        let resumed_generation =
          match public.operation {
            DomainPlanOperation::Create => 1,
            DomainPlanOperation::Update
            | DomainPlanOperation::Migrate => expected
              .checked_add(1)
              .context("GenerationConflict: generation overflow")?,
            _ => expected,
          };
        if current.config != plan.config
          || current.info.generation != resumed_generation
        {
          lease.release().await?;
          bail!(
            "StalePlan: resource changed before interrupted task could resume"
          )
        }
        update.in_progress();
        update.end_ts = None;
        update.success = true;
        update_update(update.clone()).await?;
        db_client()
          .domain_operations
          .update_one(
            doc! {"request_id":&request_id},
            doc! {"$set":{"state":"Accepted"}},
          )
          .await?;
        db_client().domains.update_one(doc!{"_id":ObjectId::parse_str(&id)?},doc!{"$set":{"info.state":"Applying","info.conditions":[]}}).await?;
        spawn_run(plan, id, lease, update.clone(), request_id);
      }
      return Ok(update);
    }
  }
  if !recovering && public.expires_at < komodo_timestamp() {
    bail!("StalePlan: plan expired")
  }
  super::policy::validate_plan_instance(
    &crate::config::core_config().domain_instance_id,
    &plan.instance,
  )?;
  match (&public.operation, kind) {
    (DomainPlanOperation::Remove, "RemoveDomain")
    | (DomainPlanOperation::Migrate, "MigrateDomain")
    | (
      DomainPlanOperation::FinalizeMigration,
      "FinalizeDomainMigration",
    ) => {}
    (
      DomainPlanOperation::Create
      | DomainPlanOperation::Update
      | DomainPlanOperation::Reconcile
      | DomainPlanOperation::Adopt
      | DomainPlanOperation::Abandon,
      "ReconcileDomain",
    ) => {}
    _ => bail!(
      "InvalidPlan: execute method does not match plan operation"
    ),
  }
  if public.operation != DomainPlanOperation::Create {
    let current = crate::domain::authorized::<Domain>(
      &id,
      &user,
      level(&public.operation).into(),
    )
    .await?;
    super::policy::validate_generation(
      current.info.generation,
      expected,
      recovering,
      current.config == plan.config,
    )?;
  } else if !user.admin
    && crate::config::core_config().disable_non_admin_create
  {
    bail!("Forbidden: domain creation disabled")
  }
  let mut checked = plan.config.clone();
  let (ingress, provider, backend) =
    validation::resolve_for_operation(
      &mut checked,
      &user,
      &public.operation,
    )
    .await?;
  if backend != plan.backend
    || ingress.info.generation != plan.ingress.info.generation
    || provider.as_ref().map(|p| p.info.generation)
      != plan.provider.as_ref().map(|p| p.info.generation)
  {
    bail!("StalePlan: connection configuration changed")
  }
  let lease = Lease::acquire(&format!("domain:{id}")).await?;
  let result=async {
  if public.operation!=DomainPlanOperation::Create {
   let current=crate::domain::authorized::<Domain>(&id,&user,level(&public.operation).into()).await?;
   super::policy::validate_generation(current.info.generation,expected,recovering,current.config==plan.config)?;
  }
  // Compare external evidence again before claiming the request.
  if public.operation!=DomainPlanOperation::Abandon && (dns_digest(&observe_dns(provider.as_ref(),&plan.config.hostname).await?)!=dns_digest(&plan.dns_observed) || digest(&observe_ingress(&ingress,&id).await?)?!=digest(&plan.ingress_observed)?) {bail!("Drifted: external configuration changed; replan")}
  lease.renew().await?;
  if !recovering { db_client().domain_operations.insert_one(doc!{"request_id":&request_id,"digest":&request_digest,"domain_id":&id,"plan_id":&public.plan_id,"state":"Admitting"}).await?; }
  if public.operation==DomainPlanOperation::Create {
   if let Some(current)=db_client().domains.find_one(doc!{"_id":ObjectId::parse_str(&id)?}).await? {
    crate::domain::authorized::<Domain>(&id,&user,PermissionLevel::Write.into()).await?;
    if current.config!=plan.config || current.info.generation!=1 {bail!("StalePlan: reserved creation resource changed")}
   }
   let mut created=Domain{name:public.name.clone(),id:id.clone(),config:plan.config.clone(),..Default::default()};
   created.info.generation=1;created.info.display_hostname=plan.display_hostname.clone();created.info.config_digest=digest(&created.config)?;
   if db_client().domains.find_one(doc!{"_id":ObjectId::parse_str(&id)?}).await?.is_none(){db_client().domains.insert_one(&created).await?;}
   crate::helpers::create_permission(&user,ResourceTarget::Domain(id.clone()),PermissionLevel::Write, Default::default()).await;
  }else if public.operation==DomainPlanOperation::Update || public.operation==DomainPlanOperation::Migrate {
   let current=resource::get::<Domain>(&id).await?;
   if !(recovering && current.config==plan.config && Some(current.info.generation)==expected.checked_add(1)) {
   let result=db_client().domains.update_one(doc!{"_id":ObjectId::parse_str(&id)?,"info.generation":expected},doc!{"$set":{"config":to_bson(&plan.config)?,"info.config_digest":digest(&plan.config)?,"info.display_hostname":&plan.display_hostname},"$inc":{"info.generation":1_i64}}).await?;
   if result.matched_count!=1 {bail!("GenerationConflict: domain changed")}
   }
  }
  let operation=match public.operation{DomainPlanOperation::Remove=>Operation::RemoveDomain,DomainPlanOperation::Migrate=>Operation::MigrateDomain,DomainPlanOperation::FinalizeMigration=>Operation::FinalizeDomainMigration,_=>Operation::ReconcileDomain};
  let mut update=make_update(ResourceTarget::Domain(id.clone()),operation,&user);
  update.in_progress();
  update.id=add_update(update.clone()).await?;
  db_client().domain_operations.update_one(doc!{"request_id":&request_id},doc!{"$set":{"update_id":&update.id,"state":"Accepted"}}).await?;
  db_client().domains.update_one(doc!{"_id":ObjectId::parse_str(&id)?},doc!{"$set":{"info.state":if public.operation==DomainPlanOperation::Remove {"Deleting"}else{"Applying"},"info.last_operation_id":&update.id}}).await?;
  anyhow::Ok(update)
 }.await;
  let update = match result {
    Ok(update) => update,
    Err(error) => {
      lease.release().await?;
      return Err(error);
    }
  };
  let accepted = update.clone();
  spawn_run(plan, id, lease, update, request_id);
  Ok(accepted)
}
fn spawn_run(
  plan: SavedPlan,
  id: String,
  lease: Lease,
  update: Update,
  request_id: String,
) {
  tokio::spawn(async move {
    let mut update = update;
    update.in_progress();
    let _ = update_update(update.clone()).await;
    let result = run(&plan, &id, &lease).await;
    let succeeded = result.is_ok();
    let mut retryable = false;
    match result {
      Ok(()) => {
        update.push_simple_log("Domain", "Operation completed; inspect independent readiness conditions");
      }
      Err(error) => {
        let message = error.to_string();
        let reported =
          message.split(':').next().unwrap_or("ExecutionFailed");
        let code = if [
          "Drifted",
          "OwnershipConflict",
          "LeaseLost",
          "RateLimited",
          "Timeout",
          "IngressUnavailable",
          "IngressLoadFailed",
          "Forbidden",
          "UnsupportedCapability",
          "VerificationPending",
          "DrainPending",
          "CredentialMissing",
          "ProviderUnavailable",
          "OwnershipUnconfirmed",
        ]
        .contains(&reported)
        {
          reported
        } else {
          "ExecutionFailed"
        }
        .to_owned();
        retryable = matches!(
          code.as_str(),
          "Timeout"
            | "IngressUnavailable"
            | "RateLimited"
            | "ProviderUnavailable"
        );
        // Never log external raw responses, backend URLs, or arbitrary error chains.
        let safe=DomainCondition{condition:DomainConditionType::IngressReady,status:ConditionStatus::False,reason:code.clone(),message:"Domain operation stopped; inspect connection health and create a new plan if drifted".into(),observed_at:komodo_timestamp()};
        let _=db_client().domains.update_one(doc!{"_id":ObjectId::parse_str(&id).unwrap()},doc!{"$set":{"info.state":if plan.public.operation==DomainPlanOperation::Remove {"Deleting"}else if code=="IngressLoadFailed" {"Degraded"}else{"Blocked"},"info.conditions":to_bson(&vec![safe]).unwrap()}}).await;
        update.push_error_log("Domain",format!("{code}: operation stopped; external objects retained for recovery"));
      }
    }
    update.finalize();
    update.success = succeeded;
    let _ = update_update(update).await;
    let _ = db_client()
      .domain_operations
      .update_one(
        doc! {"request_id":&request_id},
        doc! {"$set":{"state":if retryable {"Retryable"}else{"Complete"}}},
      )
      .await;
    let _ = lease.release().await;
  });
}

async fn run(
  plan: &SavedPlan,
  id: &str,
  lease: &Lease,
) -> anyhow::Result<()> {
  let mut owned = state(id).await?;
  super::policy::validate_ownership_instance(
    &plan.instance,
    &owned.instance,
    owned.dns.is_some()
      || owned.ingress.is_some()
      || owned.old.is_some(),
  )?;
  owned.instance = plan.instance.clone();
  owned.operator = plan.author.clone();
  let shared_key = match &plan.ingress.config.adapter {
    IngressAdapterConfig::CloudflareTunnelV1 {
      account_id,
      tunnel_id,
      ..
    } => format!("tunnel:{account_id}:{tunnel_id}"),
    _ => format!("ingress:{}", plan.ingress.id),
  };
  let result=async {
  lease.renew().await?;
  if plan.public.operation==DomainPlanOperation::Abandon {
   let mut record=database::bson::to_document(&owned)?;record.insert("domain_id",id);record.insert("abandoned_at",komodo_timestamp());
   db_client().db.collection::<Document>("DomainAbandoned").insert_one(record).await?;
   owned.dns=None;owned.ingress=None;owned.old=None;owned.pending_dns=None;owned.pending_tunnel=None;owned.cleaned=true;owned.references.clear();save(id,&owned).await?;
   return finish(id,"Abandoned",false).await;
  }
  if plan.public.operation==DomainPlanOperation::FinalizeMigration {
   if owned.cutover_at==0 || komodo_timestamp()-owned.cutover_at < i64::from(plan.ingress.config.drain_seconds)*1000 {bail!("DrainPending: migration observation period has not elapsed")}
   verify_public(plan).await?;
   let result=if let Some(old)=owned.old.as_mut(){remove_owned(id,old,lease).await}else{Ok(())};
   save(id,&owned).await?;result?;
   owned.old=None;owned.references=references(plan);save(id,&owned).await?;
   return finish(id,"MigrationFinalized",false).await;
  }
  if plan.public.operation==DomainPlanOperation::Remove {
   let result=remove_owned(id,&mut owned,lease).await;save(id,&owned).await?;result?;
   let result=if let Some(old)=owned.old.as_mut(){remove_owned(id,old,lease).await}else{Ok(())};save(id,&owned).await?;result?;
   owned.old=None;owned.cleaned=true;owned.references.clear();save(id,&owned).await?;
   return finish(id,"Removed",false).await;
  }
  let shared=Lease::acquire(&shared_key).await?;
  let mutation=async {
  if plan.public.operation==DomainPlanOperation::Migrate && owned.old.is_none() {
   let old=owned.clone();owned.old=Some(Box::new(old));owned.migration_plan=plan.public.plan_id.clone();owned.dns=None;owned.ingress=None;
   // Same DNS connection must retain its precise owned record across ingress switch.
   if let Some(old)=&owned.old && old.config.as_ref().is_some_and(|c|provider_ref(&c.dns)==provider_ref(&plan.config.dns)) {owned.dns=old.dns.clone();}
  }
  if plan.public.operation==DomainPlanOperation::Migrate {
   if owned.migration_plan!=plan.public.plan_id {bail!("MigrationInProgress: finalize or retry the existing migration before planning another")}
   if let Some(old)=owned.old.as_mut() {
    if old.config.as_ref().is_some_and(|c|c.ingress==plan.config.ingress) {owned.ingress=old.ingress.take();}
    if old.config.as_ref().is_some_and(|c|provider_ref(&c.dns)==provider_ref(&plan.config.dns)) {old.dns=None;}
   }
  }
  if plan.public.operation==DomainPlanOperation::Migrate {
   if let Some(old)=owned.old.as_mut() {
    if let Some(old_config)=&old.config {
     if let (Some(old_provider_id),Some(provider))=(provider_ref(&old_config.dns),plan.provider.as_ref()) {
      let old_provider=resource::get::<DnsProvider>(old_provider_id).await?;
      if old_provider.config.account_id==provider.config.account_id && old_provider.config.zone_id==provider.config.zone_id && owned.dns.is_none(){owned.dns=old.dns.take();}
     }
     let old_ingress=resource::get::<Ingress>(&old_config.ingress).await?;
     if same_ingress(&old_ingress,&plan.ingress) && owned.ingress.is_none(){owned.ingress=old.ingress.take();}
    }
   }
  }
  owned.resolved_backend=plan.backend.clone();
  owned.config=Some(plan.config.clone());owned.references=references(plan);
  if let Some(old)=&owned.old {owned.references.extend(old.references.clone());}
  owned.cleaned=false;save(id,&owned).await?;
  if plan.public.operation==DomainPlanOperation::Adopt {
   for object in &plan.objects {
    if let Some(found)=plan.dns_observed.as_array().and_then(|a|a.iter().find(|r|r["id"].as_str()==Some(object))) {
     let provider=plan.provider.as_ref().context("InvalidPlan: DNS connection missing")?;
     lease.renew().await?;shared.renew().await?;
     owned.dns=Some(DnsDriver::from_provider(provider)?.adopt(object,&dns_fingerprint(found),&format!("komodo:{}:{id}",plan.instance)).await?);
    } else if matches!(plan.ingress.config.adapter,IngressAdapterConfig::CloudflareTunnelV1{..}) && object==&plan.config.hostname {
     let current=observe_ingress(&plan.ingress,id).await?;
     if fingerprint(&current)!=fingerprint(&plan.ingress_observed){bail!("Drifted: Tunnel changed before explicit adoption")}
     let rule=current["ingress"].as_array().and_then(|a|a.iter().find(|r|r["hostname"]==*object)).context("InvalidPlan: adoption rule missing")?;
     owned.ingress=Some(adopt_tunnel_rule(&current,object,&fingerprint(rule),id)?);
    } else {bail!("UnsupportedCapability: object cannot be adopted by this connection")}
    save(id,&owned).await?;
   }
  }
  let route=RouteSpec{binding_id:id.into(),hostname:plan.config.hostname.clone(),backend_url:plan.backend.clone()};
  lease.renew().await?;shared.renew().await?;
  let driver=IngressDriver::from_ingress(&plan.ingress).await?;
  let current=driver.observe(id).await?;
  check_tunnel_baseline(&plan.ingress,&current,owned.pending_tunnel.as_ref().map(|(_,post)|post.as_str()),plan.public.operation==DomainPlanOperation::Adopt).await?;
  if matches!(plan.ingress.config.adapter,IngressAdapterConfig::CloudflareTunnelV1{..}) && fingerprint(&current)!=fingerprint(&plan.ingress_observed) && !owned.pending_tunnel.as_ref().is_some_and(|(_,post)|*post==fingerprint(&current)){bail!("Drifted: shared Tunnel configuration changed since planning")}
  if let Some((handle,post))=&owned.pending_tunnel {
   if fingerprint(&current)==*post {owned.ingress=Some(handle.clone());owned.pending_tunnel=None;save(id,&owned).await?;}
  }
  let ingress_plan=driver.plan(&route,&plan.config.tls,&current,owned.ingress.as_ref())?;
  if plan.public.operation==DomainPlanOperation::Migrate && ingress_plan.requires_downtime!=plan.public.requires_downtime {bail!("StalePlan: TLS bootstrap dependencies changed")}
  if let Some(handle)=ingress_plan.handle {owned.pending_tunnel=Some((handle,fingerprint(&ingress_plan.desired)));save(id,&owned).await?;}
  lease.renew().await?;shared.renew().await?;
  owned.ingress=Some(driver.apply(&route,&plan.config.tls,owned.ingress.as_ref(),&current,shared.fence as u64).await?);
  owned.pending_tunnel=None;
  let after=driver.observe(id).await?;
  save_tunnel_baseline(&plan.ingress,&after).await?;

  save(id,&owned).await?;
  if plan.public.operation==DomainPlanOperation::Migrate && !plan.public.requires_downtime {
   super::observation::verify_candidate(&plan.config,&plan.ingress).await?;
  }
  if let Some(provider)=&plan.provider {
   let (ttl,proxied)=match plan.config.dns{DomainDns::Managed{ttl,ref proxy_mode,..}=>(ttl,*proxy_mode==ProxyMode::Proxied),_=>unreachable!()};
   let content=ingress_plan.dns_target.clone();
   let record_type=match content.parse::<std::net::IpAddr>(){Ok(std::net::IpAddr::V4(_))=>"A",Ok(std::net::IpAddr::V6(_))=>"AAAA",_=>"CNAME"};
   lease.renew().await?;shared.renew().await?;
   let driver=DnsDriver::from_provider(provider)?;
   let marker=format!("komodo:{}:{id}",plan.instance);
   let records=driver.observe(&plan.config.hostname).await?;
   if let Some(expected)=&owned.pending_dns {
    if let Some(found)=records.iter().find(|r|dns_fingerprint(r)==*expected && r["comment"]==marker) {
     owned.dns=Some(OwnedObject{id:found["id"].as_str().context("ProviderResponse: missing DNS ID")?.into(),owner:marker.clone(),fingerprint:expected.clone()});
    }
   }
   let intent=DnsIntent{hostname:plan.config.hostname.clone(),record_type:record_type.into(),content,ttl,proxied};
   let dns_plan=driver.plan(&intent,&records,owned.dns.as_ref(),&marker)?;
   owned.pending_dns=Some(dns_plan.fingerprint);save(id,&owned).await?;
   lease.renew().await?;shared.renew().await?;
   if dns_plan.changes || owned.dns.is_none(){owned.dns=Some(driver.apply(&intent,owned.dns.as_ref(),&marker).await?);}
   owned.pending_dns=None;
   save(id,&owned).await?;
  }
  if plan.public.operation==DomainPlanOperation::Migrate {owned.cutover_at=komodo_timestamp();save(id,&owned).await?;}
  // Publication alone does not establish TLS, network or Traefik load readiness.
  finish(id,"PublishedAwaitingVerification",false).await
  }.await;
  shared.release().await?;
  mutation
 }.await;
  result
}
async fn remove_owned(
  id: &str,
  owned: &mut Ownership,
  lease: &Lease,
) -> anyhow::Result<()> {
  let Some(config) = owned.config.clone() else {
    return Ok(());
  };
  if let Some(handle) = &owned.dns {
    let provider_id = provider_ref(&config.dns)
      .context("OwnershipInvalid: DNS provider missing")?;
    let provider = resource::get::<DnsProvider>(provider_id).await?;
    lease.renew().await?;
    DnsDriver::from_provider(&provider)?
      .remove_owned(handle)
      .await?;
    owned.dns = None;
  }
  let ingress = resource::get::<Ingress>(&config.ingress).await?;
  let key = match &ingress.config.adapter {
    IngressAdapterConfig::CloudflareTunnelV1 {
      account_id,
      tunnel_id,
      ..
    } => format!("tunnel:{account_id}:{tunnel_id}"),
    _ => format!("ingress:{}", ingress.id),
  };
  let shared = Lease::acquire(&key).await?;
  let result = async {
    // Preserve tombstone between DNS withdrawal and route removal. Renew domain
    // lease during drain. Bound to a maximum supported drain duration.
    let mut remaining = ingress.config.drain_seconds as u64;
    while remaining > 0 {
      let interval = remaining.min(20);
      lease.renew().await?;
      shared.renew().await?;
      tokio::time::sleep(std::time::Duration::from_secs(interval))
        .await;
      remaining -= interval;
    }
    if let Some(handle) = &owned.ingress {
      lease.renew().await?;
      shared.renew().await?;
      let driver=IngressDriver::from_ingress(&ingress).await?;
      let current=driver.observe(id).await?;
      let journal_key=format!("remove:{id}:{}",ingress.id);
      let journal=db_client().db.collection::<Document>("DomainIngressIntent");
      let prior=journal.find_one(doc!{"_id":&journal_key,"instance":&owned.instance}).await?;
      let pending=prior.as_ref().and_then(|d|d.get_str("post").ok());
      check_tunnel_baseline(&ingress,&current,pending,false).await?;
      let next=driver.plan_remove(id,handle,&current)?;
      journal.replace_one(doc!{"_id":&journal_key},doc!{"_id":&journal_key,"instance":&owned.instance,"post":fingerprint(&next)}).upsert(true).await?;
      lease.renew().await?;shared.renew().await?;
      driver.remove_owned(id,handle,&current,shared.fence as u64).await?;
      let after=driver.observe(id).await?;
      save_tunnel_baseline(&ingress,&after).await?;
      journal.delete_one(doc!{"_id":&journal_key}).await?;
      owned.ingress = None;
    }
    owned.cleaned = true;
    anyhow::Ok(())
  }
  .await;
  shared.release().await?;
  result
}
async fn finish(
  id: &str,
  reason: &str,
  ready: bool,
) -> anyhow::Result<()> {
  let current = resource::get::<Domain>(id).await?;
  let removed = matches!(reason, "Removed" | "Abandoned");
  let conditions=[DomainConditionType::TargetResolved,DomainConditionType::IngressReady,DomainConditionType::DnsConfigured,DomainConditionType::TlsReady,DomainConditionType::Reachable].into_iter().map(|condition|DomainCondition{status:if ready {ConditionStatus::True}else{ConditionStatus::Unknown},condition,reason:reason.into(),message:if removed{"External ownership cleared; record retained for audit"}else{"Configuration published; ingress load, DNS propagation, TLS and reachability require independent verification"}.into(),observed_at:komodo_timestamp()}).collect::<Vec<_>>();
  db_client().domains.update_one(doc!{"_id":ObjectId::parse_str(id)?},doc!{"$set":{"info.observed_generation":current.info.generation,"info.state":if ready{"Ready"}else if removed{"Pending"}else{"Degraded"},"info.conditions":to_bson(&conditions)?}}).await?;
  Ok(())
}
async fn verify_public(plan: &SavedPlan) -> anyhow::Result<()> {
  let conditions = super::observation::observe_readiness(
    &plan.config,
    &plan.ingress,
    plan.public.domain_id.as_deref().unwrap_or(""),
  )
  .await;
  if !conditions.iter().all(|c| c.status == ConditionStatus::True) {
    bail!(
      "VerificationPending: current DNS, ingress, TLS and public reachability must be independently verified before old resources are removed"
    )
  }
  Ok(())
}

/// A process restart never silently declares lost tasks successful. The same
/// immutable request can resume after explicit caller authorization is checked.
pub async fn recover_interrupted() -> anyhow::Result<()> {
  db_client()
    .domain_operations
    .update_many(
      doc! {"state":{"$in":["Accepted","Admitting"]}},
      doc! {"$set":{"state":"Interrupted"}},
    )
    .await?;
  db_client().domains.update_many(doc!{"info.state":"Applying"},doc!{"$set":{"info.state":"Blocked","info.conditions":to_bson(&vec![DomainCondition{condition:DomainConditionType::IngressReady,status:ConditionStatus::False,reason:"OperationInterrupted".into(),message:"Controller restarted. Retry the same plan and request ID to resume after ownership observation".into(),observed_at:komodo_timestamp()}])?}}).await?;
  Ok(())
}

/// Automatic mode only revisits explicitly established ownership, under the
/// original caller's current permissions. It never discovers or adopts objects.
pub fn start_automatic() {
  tokio::spawn(async {
    let mut timer =
      tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
      timer.tick().await;
      let Ok(domains)=database::mungos::find::find_collect(&db_client().domains,doc!{"config.reconcile_mode":"Automatic","info.state":{"$in":["Ready","Degraded"]}},None).await else {continue};
      for domain in domains {
        let Ok(owned) = state(&domain.id).await else {
          continue;
        };
        if owned.cleaned
          || owned.operator.is_empty()
          || owned.instance
            != crate::config::core_config().domain_instance_id
        {
          continue;
        }
        let Ok(Some(user)) = database::mungos::by_id::find_one_by_id(
          &db_client().users,
          &owned.operator,
        )
        .await
        else {
          continue;
        };
        if !user.enabled {
          continue;
        }
        if let Ok(ingress) =
          resource::get::<Ingress>(&domain.config.ingress).await
        {
          if let IngressAdapterConfig::CloudflareTunnelV1 {
            account_id,
            tunnel_id,
            ..
          } = &ingress.config.adapter
          {
            let key = format!("tunnel:{account_id}:{tunnel_id}");
            let Ok(Some(baseline)) = db_client()
              .db
              .collection::<Document>("DomainIngressState")
              .find_one(doc! {"_id":key})
              .await
            else {
              continue;
            };
            let Ok(current) =
              observe_ingress(&ingress, &domain.id).await
            else {
              continue;
            };
            if baseline.get_str("instance")
              != Ok(
                crate::config::core_config()
                  .domain_instance_id
                  .as_str(),
              )
              || baseline.get_str("fingerprint")
                != Ok(fingerprint(&current).as_str())
            {
              let condition=DomainCondition{condition:DomainConditionType::IngressReady,status:ConditionStatus::False,reason:"Drifted".into(),message:"Shared Tunnel changed outside the controller; explicitly review a new plan".into(),observed_at:komodo_timestamp()};
              if let Ok(id) = ObjectId::parse_str(&domain.id) {
                let _=db_client().domains.update_one(doc!{"_id":id,"info.generation":domain.info.generation},doc!{"$set":{"info.state":"Blocked","info.conditions":to_bson(&vec![condition]).unwrap_or_default()}}).await;
              }
              continue;
            }
          }
        } else {
          continue;
        }

        let _ = accept(
          domain.id,
          String::new(),
          0,
          String::new(),
          "ReconcileDomain",
          user,
        )
        .await;
      }
    }
  });
}

pub(super) async fn check_plan_objects(
  id: &str,
  operation: &DomainPlanOperation,
  dns: &serde_json::Value,
  ingress: &Ingress,
  observed: &serde_json::Value,
  hostname: &str,
  objects: &[String],
) -> anyhow::Result<()> {
  let mut owned = state(id).await?;
  if *operation != DomainPlanOperation::Abandon {
    let pending_delete=db_client().db.collection::<Document>("DomainIngressIntent").find_one(doc!{"_id":format!("remove:{id}:{}",ingress.id),"instance":&crate::config::core_config().domain_instance_id}).await?;
    let pending = owned
      .pending_tunnel
      .as_ref()
      .map(|(_, post)| post.as_str())
      .or_else(|| {
        pending_delete.as_ref().and_then(|d| d.get_str("post").ok())
      });
    check_tunnel_baseline(
      ingress,
      observed,
      pending,
      *operation == DomainPlanOperation::Adopt,
    )
    .await?;
  }
  if owned.old.is_some()
    && matches!(
      operation,
      DomainPlanOperation::Migrate | DomainPlanOperation::Update
    )
  {
    bail!(
      "MigrationInProgress: finalize existing migration before changing desired state"
    )
  }
  if owned.cleaned
    && !matches!(
      operation,
      DomainPlanOperation::Adopt
        | DomainPlanOperation::Remove
        | DomainPlanOperation::Abandon
    )
  {
    bail!(
      "OwnershipAbandoned: cleaned bindings require explicit adoption or a new resource"
    )
  }
  if let Some(config) = &owned.config {
    if config.ingress != ingress.id {
      let old_ingress =
        resource::get::<Ingress>(&config.ingress).await?;
      if !same_ingress(&old_ingress, ingress) {
        owned.ingress = None;
      }
    }
  }
  if matches!(
    operation,
    DomainPlanOperation::Adopt
      | DomainPlanOperation::Abandon
      | DomainPlanOperation::Remove
      | DomainPlanOperation::FinalizeMigration
  ) {
    return Ok(());
  }
  if let Some(records) = dns.as_array() {
    let records: Vec<_> = records
      .iter()
      .filter(|r| {
        matches!(r["type"].as_str(), Some("A" | "AAAA" | "CNAME"))
      })
      .collect();
    if records.len() > 1 {
      bail!(
        "OwnershipConflict: multiple address records require explicit individual management"
      )
    }
    for record in records {
      let marker = format!(
        "komodo:{}:{id}",
        crate::config::core_config().domain_instance_id
      );
      if record["comment"] != marker {
        bail!(
          "OwnershipConflict: DNS record is not owned; use an explicit adopt plan"
        )
      }
      if let Some(handle) = &owned.dns
        && handle.id == record["id"].as_str().unwrap_or("")
        && handle.fingerprint != dns_fingerprint(record)
        && owned.pending_dns.as_deref()
          != Some(dns_fingerprint(record).as_str())
      {
        bail!("Drifted: owned DNS record changed outside Komodo")
      }
    }
  }
  match &ingress.config.adapter {
    IngressAdapterConfig::TraefikFileV1 { .. } => {
      if observed["exists"] == true && owned.ingress.is_none() {
        bail!("OwnershipConflict: unexpected Traefik route exists")
      }
      if let Some(handle) = &owned.ingress
        && observed["fingerprint"] != handle.fingerprint
      {
        bail!("Drifted: owned Traefik file changed")
      }
    }
    IngressAdapterConfig::CloudflareTunnelV1 { .. } => {
      if let Some(rule) = observed["ingress"]
        .as_array()
        .and_then(|a| a.iter().find(|r| r["hostname"] == hostname))
      {
        let valid = owned.ingress.as_ref().is_some_and(|h| {
          h.id == hostname && h.fingerprint == fingerprint(rule)
        }) || owned
          .pending_tunnel
          .as_ref()
          .is_some_and(|(_, post)| *post == fingerprint(observed));
        if !valid {
          bail!(
            "OwnershipConflict: Tunnel hostname is not owned or has drifted"
          )
        }
      }
    }
  }
  let _ = objects;
  Ok(())
}

fn references(plan: &SavedPlan) -> Vec<String> {
  let mut refs = vec![plan.ingress.id.clone()];
  if let Some(provider) = &plan.provider {
    refs.push(provider.id.clone());
  }
  if let IngressAdapterConfig::TraefikFileV1 { server, .. } =
    &plan.ingress.config.adapter
  {
    refs.push(server.clone());
  }
  match &plan.config.target {
    DomainTarget::Deployment { id, .. } => refs.push(id.clone()),
    DomainTarget::StackService { stack_id, .. } => {
      refs.push(stack_id.clone())
    }
    _ => {}
  }
  refs
}

fn same_ingress(a: &Ingress, b: &Ingress) -> bool {
  if a.id == b.id {
    return true;
  }
  match (&a.config.adapter, &b.config.adapter) {
    (
      IngressAdapterConfig::CloudflareTunnelV1 {
        account_id: a,
        tunnel_id: at,
        ..
      },
      IngressAdapterConfig::CloudflareTunnelV1 {
        account_id: b,
        tunnel_id: bt,
        ..
      },
    ) => a == b && at == bt,
    _ => false,
  }
}
