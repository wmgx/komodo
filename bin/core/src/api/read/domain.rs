use komodo_client::{
  api::read::*,
  entities::{
    domain::{Domain, DomainListItem, DomainPlan},
    domain_account::{
      DomainAccount, DomainDiscovery, DomainProviderKind,
    },
    permission::PermissionLevel,
  },
};
use mogh_error::AddStatusCodeError;
use mogh_resolver::Resolve;
use reqwest::StatusCode;

use crate::{
  domain::{
    accounts,
    prepare::plan_domain,
    provider::{REGISTRY, factory},
  },
  helpers::query::get_all_tags,
  permission::get_check_permissions,
  resource,
};

use super::{ReadArgs, list_limit};

impl Resolve<ReadArgs> for GetDomain {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<Domain> {
    Ok(
      get_check_permissions::<Domain>(
        &self.domain,
        user,
        PermissionLevel::Read.into(),
      )
      .await?,
    )
  }
}

impl Resolve<ReadArgs> for ListDomains {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<Vec<DomainListItem>> {
    let all_tags = if self.query.tags.is_empty() {
      vec![]
    } else {
      get_all_tags(None).await?
    };
    let limit = list_limit(self.limit);
    Ok(
      resource::list_items_for_user::<Domain>(
        self.query,
        resource::ListItemsQueryOptions {
          limit,
          page: self.page,
          sort_desc: false,
          sort_by: resource::ListItemSort::Name,
        },
        user,
        PermissionLevel::Read.into(),
        &all_tags,
        |_| true,
      )
      .await?,
    )
  }
}

impl Resolve<ReadArgs> for ListFullDomains {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<ListFullDomainsResponse> {
    let all_tags = if self.query.tags.is_empty() {
      vec![]
    } else {
      get_all_tags(None).await?
    };
    let limit = list_limit(self.limit);
    Ok(
      resource::list_full_for_user::<Domain>(
        self.query,
        limit as i64,
        self.page.saturating_mul(limit),
        user,
        PermissionLevel::Read.into(),
        &all_tags,
      )
      .await?,
    )
  }
}

impl Resolve<ReadArgs> for GetDomainPlan {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<DomainPlan> {
    let domain = get_check_permissions::<Domain>(
      &self.domain,
      user,
      PermissionLevel::Read.into(),
    )
    .await?;
    plan_domain(&domain).await.map_err(|e| e.into_api())
  }
}

impl Resolve<ReadArgs> for ListDomainProviderKinds {
  async fn resolve(
    self,
    _: &ReadArgs,
  ) -> mogh_error::Result<Vec<DomainProviderKind>> {
    Ok(
      REGISTRY
        .iter()
        .map(|f| DomainProviderKind {
          kind: f.kind.to_string(),
          capabilities: f.capabilities.to_vec(),
          params_example: (f.params_example)(),
        })
        .collect(),
    )
  }
}

impl Resolve<ReadArgs> for ListDomainAccounts {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<Vec<DomainAccount>> {
    if !user.admin {
      return Err(
        anyhow::anyhow!("Only admins can read domain accounts")
          .status_code(StatusCode::FORBIDDEN),
      );
    }
    Ok(
      accounts::list_all()
        .await
        .map_err(|e| e.into_api())?
        .iter()
        .map(|a| a.view())
        .collect(),
    )
  }
}

impl Resolve<ReadArgs> for GetDomainAccount {
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<DomainAccount> {
    if !user.admin {
      return Err(
        anyhow::anyhow!("Only admins can read domain accounts")
          .status_code(StatusCode::FORBIDDEN),
      );
    }
    accounts::find_by_id_or_name(&self.account)
      .await
      .map_err(|e| e.into_api())?
      .map(|a| a.view())
      .ok_or_else(|| {
        anyhow::anyhow!(
          "No domain account matching '{}'",
          self.account
        )
        .status_code(StatusCode::NOT_FOUND)
      })
  }
}

impl Resolve<ReadArgs> for DiscoverDomainAccount {
  #[instrument(
    "DiscoverDomainAccount",
    skip_all,
    fields(operator = user.id)
  )]
  async fn resolve(
    self,
    ReadArgs { user }: &ReadArgs,
  ) -> mogh_error::Result<DomainDiscovery> {
    if !user.admin {
      return Err(
        anyhow::anyhow!("Only admins can read domain accounts")
          .status_code(StatusCode::FORBIDDEN),
      );
    }
    let bad = |msg: &str| {
      anyhow::anyhow!("{msg}").status_code(StatusCode::BAD_REQUEST)
    };
    let (kind, token) = match self.account.filter(|a| !a.is_empty()) {
      Some(account) => {
        let account = accounts::find_by_id_or_name(&account)
          .await
          .map_err(|e| e.into_api())?
          .ok_or_else(|| {
            anyhow::anyhow!("No domain account matching '{account}'")
              .status_code(StatusCode::NOT_FOUND)
          })?;
        (account.kind, account.token)
      }
      None => (
        self.kind.unwrap_or_default(),
        self.token.unwrap_or_default(),
      ),
    };
    if token.is_empty() {
      return Err(bad("Provide a saved account, or kind and token"));
    }
    let factory = factory(&kind).ok_or_else(|| {
      bad(&format!("Unknown provider kind '{kind}'"))
    })?;
    let Some(discover) = factory.discover else {
      return Err(bad(&format!(
        "Provider kind '{kind}' does not support discovery"
      )));
    };
    discover(token).await.map_err(|e| {
      e.log("DiscoverDomainAccount");
      e.into_api()
    })
  }
}
