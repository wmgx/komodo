use database::mungos::mongodb::Collection;
use komodo_client::entities::{
  Operation, ResourceTarget, ResourceTargetVariant,
  domain::{
    Domain, DomainConfig, DomainConfigDiff, DomainInfo,
    DomainListItem, DomainListItemInfo, DomainQuerySpecifics,
    PartialDomainConfig,
  },
  resource::Resource,
  update::Update,
  user::User,
};

use crate::{
  domain::{apply::cleanup, validate::validate_partial},
  state::db_client,
};

impl super::KomodoResource for Domain {
  type Config = DomainConfig;
  type PartialConfig = PartialDomainConfig;
  type ConfigDiff = DomainConfigDiff;
  type Info = DomainInfo;
  type ListItem = DomainListItem;
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

  async fn to_list_item(
    domain: Resource<Self::Config, Self::Info>,
  ) -> Self::ListItem {
    DomainListItem {
      name: domain.name,
      id: domain.id,
      template: domain.template,
      tags: domain.tags,
      resource_type: ResourceTargetVariant::Domain,
      info: DomainListItemInfo {
        hostname: domain.config.hostname,
        dns_account: domain.config.dns.account,
        ingress_account: domain.config.ingress.account,
        target: domain.config.target,
        service: domain.config.service,
        port: domain.config.port,
        status_code: domain
          .info
          .status
          .as_ref()
          .map(|s| s.code())
          .unwrap_or("unchecked")
          .to_string(),
      },
    }
  }

  async fn busy(_id: &String) -> anyhow::Result<bool> {
    Ok(false)
  }

  // CREATE

  fn create_operation() -> Operation {
    Operation::CreateDomain
  }

  /// Creating a Domain lets Komodo write public DNS, admin only.
  fn user_can_create(user: &User) -> bool {
    user.admin
  }

  async fn validate_create_config(
    config: &mut Self::PartialConfig,
    _user: &User,
  ) -> anyhow::Result<()> {
    Ok(validate_partial(config)?)
  }

  async fn post_create(
    _created: &Resource<Self::Config, Self::Info>,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }

  // UPDATE

  fn update_operation() -> Operation {
    Operation::UpdateDomain
  }

  async fn validate_update_config(
    _id: &str,
    config: &mut Self::PartialConfig,
    _user: &User,
  ) -> anyhow::Result<()> {
    Ok(validate_partial(config)?)
  }

  async fn post_update(
    _updated: &Self,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }

  // RENAME

  fn rename_operation() -> Operation {
    Operation::RenameDomain
  }

  // DELETE

  fn delete_operation() -> Operation {
    Operation::DeleteDomain
  }

  /// With `on_delete = Cleanup` removes remote objects Komodo created.
  /// If that fails, the Domain is NOT deleted.
  async fn pre_delete(
    resource: &Resource<Self::Config, Self::Info>,
    update: &mut Update,
  ) -> anyhow::Result<()> {
    let lines = cleanup(resource).await?;
    if !lines.is_empty() {
      update
        .push_simple_log("Cleanup remote objects", lines.join("\n"));
    }
    Ok(())
  }

  async fn post_delete(
    _resource: &Resource<Self::Config, Self::Info>,
    _update: &mut Update,
  ) -> anyhow::Result<()> {
    Ok(())
  }
}
