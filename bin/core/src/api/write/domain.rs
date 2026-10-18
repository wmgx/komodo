use super::WriteArgs;
use crate::{
  domain::{digest, validation},
  resource,
  state::db_client,
};
macro_rules! bail {($($arg:tt)*)=>{return Err(anyhow::anyhow!($($arg)*).into())}}
use database::bson::{doc, oid::ObjectId, to_bson};
use komodo_client::{
  api::write::*,
  entities::{domain::*, permission::PermissionLevel},
};
use mogh_resolver::Resolve;

impl Resolve<WriteArgs> for CreateDomain {
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Domain> {
    let display_hostname = self.config.hostname.clone();
    let created = resource::create::<Domain>(
      &self.name,
      self.config.into(),
      None,
      user,
    )
    .await?;
    db_client()
      .domains
      .update_one(
        doc! {"_id":ObjectId::parse_str(&created.id)?},
        doc! {"$set":{"info.display_hostname":display_hostname}},
      )
      .await?;
    Ok(resource::get::<Domain>(&created.id).await?)
  }
}
impl Resolve<WriteArgs> for CreateDnsProvider {
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<DnsProvider> {
    resource::create::<DnsProvider>(
      &self.name,
      self.config.into(),
      None,
      user,
    )
    .await
  }
}
impl Resolve<WriteArgs> for CreateIngress {
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Ingress> {
    resource::create::<Ingress>(
      &self.name,
      self.config.into(),
      None,
      user,
    )
    .await
  }
}

impl Resolve<WriteArgs> for UpdateDomain {
  async fn resolve(
    mut self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Domain> {
    validation::enabled()?;
    let old = crate::domain::authorized::<Domain>(
      &self.id,
      user,
      PermissionLevel::Write.into(),
    )
    .await?;
    let display_hostname = self.config.hostname.clone();
    validation::validate_desired(&mut self.config, user).await?;
    if old.config.ingress != self.config.ingress
      || old.config.dns != self.config.dns
        && provider_id(&old.config.dns)
          != provider_id(&self.config.dns)
    {
      bail!(
        "MigrationRequired: connection replacement requires a migration plan"
      )
    }
    if old.config.hostname != self.config.hostname {
      bail!(
        "MigrationRequired: hostname changes require removing the existing binding first"
      )
    }
    let lease = crate::domain::lease::Lease::acquire(&format!(
      "domain:{}",
      old.id
    ))
    .await?;
    let result=async {
  let result=db_client().domains.update_one(doc!{"_id":ObjectId::parse_str(&old.id)?,"info.generation":self.expected_generation,"info.state":{"$nin":["Applying","Deleting"]}},doc!{"$set":{"config":to_bson(&self.config)?,"info.config_digest":digest(&self.config)?,"info.display_hostname":display_hostname,"info.state":"Pending"},"$inc":{"info.generation":1_i64}}).await?;
  if result.matched_count!=1 {bail!("GenerationConflict: domain changed or is being reconciled")}
  anyhow::Ok(resource::get::<Domain>(&old.id).await?)
  }.await;
    lease.release().await?;
    Ok(result?)
  }
}
fn provider_id(dns: &DomainDns) -> Option<&str> {
  match dns {
    DomainDns::Managed { provider, .. } => Some(provider),
    _ => None,
  }
}
macro_rules! update_connection {
 ($request:ty,$resource:ty,$coll:ident,$validate:ident) => {
  impl Resolve<WriteArgs> for $request {async fn resolve(mut self,WriteArgs{user}:&WriteArgs)->mogh_error::Result<$resource>{
    validation::enabled()?;
    if !user.admin {bail!("Forbidden: only administrators configure connections")}
    validation::$validate(&mut self.config).await?;
    let old=crate::domain::authorized::<$resource>(&self.id,user,PermissionLevel::Write.into()).await?;
    // Connection identity/config is immutable while attached; use a replacement
    // resource and migration so ownership handles cannot silently change scope.
    if db_client().domains.count_documents(doc!{"$or":[{"config.ingress":&old.id},{"config.dns.params.provider":&old.id}]}).await?>0 || db_client().db.collection::<database::bson::Document>("DomainOwnership").count_documents(doc!{"references":&old.id}).await?>0 {bail!("ResourceInUse: create a new connection and migrate dependent domains")}
    let result=db_client().$coll.update_one(doc!{"_id":ObjectId::parse_str(&old.id)?,"info.generation":self.expected_generation},doc!{"$set":{"config":to_bson(&self.config)?},"$inc":{"info.generation":1_i64}}).await?;
    if result.matched_count!=1 {bail!("GenerationConflict: connection changed")}
    Ok(resource::get::<$resource>(&old.id).await?)
  }}
 }
}
update_connection!(
  UpdateDnsProvider,
  DnsProvider,
  dns_providers,
  normalize_provider
);
update_connection!(
  UpdateIngress,
  Ingress,
  ingresses,
  normalize_ingress
);
macro_rules! delete {
  ($request:ty,$resource:ty) => {
    impl Resolve<WriteArgs> for $request {
      async fn resolve(
        self,
        WriteArgs { user }: &WriteArgs,
      ) -> mogh_error::Result<$resource> {
        validation::enabled()?;
        Ok(resource::delete::<$resource>(&self.id, user).await?)
      }
    }
  };
}
impl Resolve<WriteArgs> for DeleteDomain {
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Domain> {
    validation::enabled()?;
    let domain = crate::domain::authorized::<Domain>(
      &self.id,
      user,
      PermissionLevel::Write.into(),
    )
    .await?;
    let lease = crate::domain::lease::Lease::acquire(&format!(
      "domain:{}",
      domain.id
    ))
    .await?;
    let result=async {
   let state=db_client().domain_ownership.find_one(doc!{"_id":&domain.id}).await?;
   if state.as_ref().is_some_and(|s|s.get_bool("cleaned")!=Ok(true)){return Err(anyhow::anyhow!("CleanupRequired: remove or abandon owned external resources first"))}
   db_client().domains.delete_one(doc!{"_id":ObjectId::parse_str(&domain.id)?}).await?;
   db_client().domain_ownership.delete_one(doc!{"_id":&domain.id}).await?;
   resource::delete_all_permissions_on_resource(komodo_client::entities::ResourceTarget::Domain(domain.id.clone())).await;
   let mut update=crate::helpers::update::make_update(komodo_client::entities::ResourceTarget::Domain(domain.id.clone()),komodo_client::entities::Operation::DeleteDomain,user);
   update.finalize();crate::helpers::update::add_update(update).await?;
   anyhow::Ok(domain)
  }.await;
    lease.release().await?;
    Ok(result?)
  }
}
delete!(DeleteDnsProvider, DnsProvider);
delete!(DeleteIngress, Ingress);
