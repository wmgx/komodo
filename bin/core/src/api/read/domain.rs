use super::{ReadArgs, list_limit};
use crate::{domain, helpers::query::get_all_tags, resource};
use komodo_client::{
  api::read::*,
  entities::{domain::*, permission::PermissionLevel},
};
use mogh_resolver::Resolve;
macro_rules! get {
  ($request:ty,$res:ty,$field:ident) => {
    impl Resolve<ReadArgs> for $request {
      async fn resolve(
        self,
        ReadArgs { user }: &ReadArgs,
      ) -> mogh_error::Result<$res> {
        domain::validation::enabled()?;
        Ok(
          crate::domain::authorized::<$res>(
            &self.$field,
            user,
            PermissionLevel::Read.into(),
          )
          .await?,
        )
      }
    }
  };
}
get!(GetDomain, Domain, domain);
get!(GetDnsProvider, DnsProvider, dns_provider);
get!(GetIngress, Ingress, ingress);
macro_rules! list {
  ($request:ty,$res:ty) => {
    impl Resolve<ReadArgs> for $request {
      async fn resolve(
        self,
        ReadArgs { user }: &ReadArgs,
      ) -> mogh_error::Result<Vec<$res>> {
        domain::validation::enabled()?;
        let tags = get_all_tags(None).await?;
        Ok(
          resource::list_full_for_user::<$res>(
            self.query,
            Some(list_limit(self.limit) as i64),
            Some(self.page.saturating_mul(list_limit(self.limit))),
            user,
            PermissionLevel::Read.into(),
            &tags,
          )
          .await?,
        )
      }
    }
  };
}
list!(ListDomains, Domain);
list!(ListDnsProviders, DnsProvider);
list!(ListIngresses, Ingress);
impl Resolve<ReadArgs> for PlanDomainChange {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<DomainPlan> {
    Ok(domain::plan(self.intent, user).await?)
  }
}
impl Resolve<ReadArgs> for GetDomainPlan {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<DomainPlan> {
    Ok(domain::load_plan(&self.plan_id, user).await?.public)
  }
}
impl Resolve<ReadArgs> for ListDomainAdapters {
  async fn resolve(
    self,
    _: &ReadArgs,
  ) -> mogh_error::Result<Vec<DomainAdapterCapabilities>> {
    domain::validation::enabled()?;
    Ok(
      [
        "CloudflareDnsV1",
        "TraefikFileV1",
        "CloudflareTunnelV1",
        "ExternalDns",
      ]
      .iter()
      .map(|a| domain::capabilities(a))
      .collect::<anyhow::Result<_>>()?,
    )
  }
}
impl Resolve<ReadArgs> for GetDomainAdapterCapabilities {
  async fn resolve(
    self,
    _: &ReadArgs,
  ) -> mogh_error::Result<DomainAdapterCapabilities> {
    domain::validation::enabled()?;
    Ok(domain::capabilities(&self.adapter)?)
  }
}
impl Resolve<ReadArgs> for ValidateDomainConnection {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<DomainConnectionValidation> {
    Ok(domain::validate_connection(self.connection, user).await?)
  }
}
impl Resolve<ReadArgs> for GetDomainStatus {
  async fn resolve(
    self,
    args: &ReadArgs,
  ) -> mogh_error::Result<DomainStatus> {
    let mut domain = GetDomain {
      domain: self.domain,
    }
    .resolve(args)
    .await?;
    domain::refresh_status(&mut domain).await?;
    let error = domain
      .info
      .conditions
      .iter()
      .find(|c| c.status == ConditionStatus::False)
      .map(|c| DomainError {
        code: c.reason.clone(),
        message: c.message.clone(),
        retryable: matches!(
          c.reason.as_str(),
          "RateLimited" | "Timeout" | "IngressUnavailable"
        ),
        field: None,
        update_id: domain.info.last_operation_id.clone(),
      });
    Ok(DomainStatus {
      domain_id: domain.id,
      generation: domain.info.generation,
      observed_generation: domain.info.observed_generation,
      state: domain.info.state,
      conditions: domain.info.conditions,
      update_id: domain.info.last_operation_id,
      error,
    })
  }
}
