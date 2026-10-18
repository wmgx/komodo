use crate::{domain::validation, state::db_client};
use anyhow::bail;
use database::{
  bson::{Document, doc},
  mungos::mongodb::Collection,
};
use komodo_client::entities::{
  Operation, ResourceTarget, ResourceTargetVariant, domain::*,
  resource::Resource, update::Update, user::User,
};
impl super::KomodoResource for Domain {
  type Config = DomainConfig;
  type PartialConfig = PartialDomainConfig;
  type ConfigDiff = DomainConfigDiff;
  type Info = DomainInfo;
  type ListItem = Domain;
  type QuerySpecifics = DomainQuerySpecifics;
  fn resource_type() -> ResourceTargetVariant {
    ResourceTargetVariant::Domain
  }
  fn resource_target(id: impl Into<String>) -> ResourceTarget {
    ResourceTarget::Domain(id.into())
  }
  fn coll() -> &'static Collection<Resource<Self::Config, Self::Info>>
  {
    &db_client().domains
  }
  async fn to_list_item(resource: Self) -> Self {
    resource
  }
  async fn busy(_id: &String) -> anyhow::Result<bool> {
    Ok(false)
  }
  fn create_operation() -> Operation {
    Operation::CreateDomain
  }
  fn update_operation() -> Operation {
    Operation::UpdateDomain
  }
  fn rename_operation() -> Operation {
    Operation::RenameDomain
  }
  fn delete_operation() -> Operation {
    Operation::DeleteDomain
  }
  fn user_can_create(user: &User) -> bool {
    user.admin
      || !crate::config::core_config().disable_non_admin_create
  }
  async fn default_info() -> anyhow::Result<Self::Info> {
    Ok(DomainInfo {
      generation: 1,
      ..Default::default()
    })
  }
  async fn validate_create_config(
    config: &mut Self::PartialConfig,
    user: &User,
  ) -> anyhow::Result<()> {
    validation::enabled()?;
    let mut full = Self::Config::from(config.clone());
    validation::validate_desired(&mut full, user).await?;
    *config = full.into();
    Ok(())
  }
  async fn validate_update_config(
    _id: &str,
    _config: &mut Self::PartialConfig,
    _user: &User,
  ) -> anyhow::Result<()> {
    bail!(
      "GenerationRequired: use the domain API with expected_generation"
    )
  }
  async fn post_create(
    resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    db_client().domains.update_one(doc!{"_id":database::bson::oid::ObjectId::parse_str(&resource.id)?},doc!{"$set":{"info.config_digest":crate::domain::digest(&resource.config)?}}).await?;
    Ok(())
  }
  async fn post_update(
    _resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
  async fn post_delete(
    _resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
  async fn pre_delete(
    resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    validation::enabled()?;
    if db_client().domain_leases.count_documents(doc! {"_id":format!("domain:{}",resource.id),"expires":{"$gt":komodo_client::entities::komodo_timestamp()}}).await?>0 {bail!("DomainBusy: a domain operation owns the lease")}
    let state = db_client()
      .db
      .collection::<Document>("DomainOwnership")
      .find_one(doc! {"_id":&resource.id})
      .await?;
    if state
      .as_ref()
      .is_some_and(|s| s.get_bool("cleaned") != Ok(true))
    {
      bail!(
        "CleanupRequired: use RemoveDomain or explicit Abandon before deletion"
      )
    }
    Ok(())
  }
}
impl super::KomodoResource for DnsProvider {
  type Config = DnsProviderConfig;
  type PartialConfig = PartialDnsProviderConfig;
  type ConfigDiff = DnsProviderConfigDiff;
  type Info = ConnectionInfo;
  type ListItem = DnsProvider;
  type QuerySpecifics = DomainQuerySpecifics;
  fn resource_type() -> ResourceTargetVariant {
    ResourceTargetVariant::DnsProvider
  }
  fn resource_target(id: impl Into<String>) -> ResourceTarget {
    ResourceTarget::DnsProvider(id.into())
  }
  fn coll() -> &'static Collection<Resource<Self::Config, Self::Info>>
  {
    &db_client().dns_providers
  }
  async fn to_list_item(resource: Self) -> Self {
    resource
  }
  async fn busy(_id: &String) -> anyhow::Result<bool> {
    Ok(false)
  }
  fn create_operation() -> Operation {
    Operation::CreateDnsProvider
  }
  fn update_operation() -> Operation {
    Operation::UpdateDnsProvider
  }
  fn rename_operation() -> Operation {
    Operation::RenameDnsProvider
  }
  fn delete_operation() -> Operation {
    Operation::DeleteDnsProvider
  }
  fn user_can_create(user: &User) -> bool {
    user.admin
  }
  async fn default_info() -> anyhow::Result<Self::Info> {
    Ok(ConnectionInfo {
      generation: 1,
      ..Default::default()
    })
  }
  async fn validate_create_config(
    config: &mut Self::PartialConfig,
    user: &User,
  ) -> anyhow::Result<()> {
    validation::enabled()?;
    let full = Self::Config::from(config.clone());
    if !user.admin {
      bail!("Forbidden: only administrators configure connections")
    }
    validation::provider(&full)?;
    *config = full.into();
    Ok(())
  }
  async fn validate_update_config(
    _id: &str,
    _config: &mut Self::PartialConfig,
    _user: &User,
  ) -> anyhow::Result<()> {
    bail!(
      "GenerationRequired: use the domain API with expected_generation"
    )
  }
  async fn post_create(
    _resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
  async fn post_update(
    _resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
  async fn post_delete(
    _resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
  async fn pre_delete(
    resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    validation::enabled()?;
    if db_client()
      .domains
      .count_documents(
        doc! {"config.dns.params.provider":&resource.id},
      )
      .await?
      > 0
      || db_client()
        .db
        .collection::<Document>("DomainOwnership")
        .count_documents(doc! {"references":&resource.id})
        .await?
        > 0
    {
      bail!(
        "ResourceInUse: connection is referenced by an active domain or migration"
      )
    }
    Ok(())
  }
}
impl super::KomodoResource for Ingress {
  type Config = IngressConfig;
  type PartialConfig = PartialIngressConfig;
  type ConfigDiff = IngressConfigDiff;
  type Info = ConnectionInfo;
  type ListItem = Ingress;
  type QuerySpecifics = DomainQuerySpecifics;
  fn resource_type() -> ResourceTargetVariant {
    ResourceTargetVariant::Ingress
  }
  fn resource_target(id: impl Into<String>) -> ResourceTarget {
    ResourceTarget::Ingress(id.into())
  }
  fn coll() -> &'static Collection<Resource<Self::Config, Self::Info>>
  {
    &db_client().ingresses
  }
  async fn to_list_item(resource: Self) -> Self {
    resource
  }
  async fn busy(_id: &String) -> anyhow::Result<bool> {
    Ok(false)
  }
  fn create_operation() -> Operation {
    Operation::CreateIngress
  }
  fn update_operation() -> Operation {
    Operation::UpdateIngress
  }
  fn rename_operation() -> Operation {
    Operation::RenameIngress
  }
  fn delete_operation() -> Operation {
    Operation::DeleteIngress
  }
  fn user_can_create(user: &User) -> bool {
    user.admin
  }
  async fn default_info() -> anyhow::Result<Self::Info> {
    Ok(ConnectionInfo {
      generation: 1,
      ..Default::default()
    })
  }
  async fn validate_create_config(
    config: &mut Self::PartialConfig,
    user: &User,
  ) -> anyhow::Result<()> {
    validation::enabled()?;
    let mut full = Self::Config::from(config.clone());
    if !user.admin {
      bail!("Forbidden: only administrators configure connections")
    }
    validation::ingress(&full)?;
    if let IngressAdapterConfig::TraefikFileV1 { server, .. } =
      &mut full.adapter
    {
      *server =
        super::get::<komodo_client::entities::server::Server>(server)
          .await?
          .id;
    }
    *config = full.into();
    Ok(())
  }
  async fn validate_update_config(
    _id: &str,
    _config: &mut Self::PartialConfig,
    _user: &User,
  ) -> anyhow::Result<()> {
    bail!(
      "GenerationRequired: use the domain API with expected_generation"
    )
  }
  async fn post_create(
    _resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
  async fn post_update(
    _resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
  async fn post_delete(
    _resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
  async fn pre_delete(
    resource: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    validation::enabled()?;
    if db_client()
      .domains
      .count_documents(doc! {"config.ingress":&resource.id})
      .await?
      > 0
      || db_client()
        .db
        .collection::<Document>("DomainOwnership")
        .count_documents(doc! {"references":&resource.id})
        .await?
        > 0
    {
      bail!(
        "ResourceInUse: connection is referenced by an active domain or migration"
      )
    }
    Ok(())
  }
}
