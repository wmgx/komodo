use std::fmt;

use mogh_resolver::Resolve;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::entities::{
  U64,
  domain::{Domain, DomainListItem, DomainPlan, DomainQuery},
  domain_account::{
    DomainAccount, DomainDiscovery, DomainProviderKind,
  },
};

use super::KomodoReadRequest;

//

/// Get a specific domain. Response: [Domain].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoReadRequest)]
#[response(GetDomainResponse)]
#[error(mogh_error::Error)]
pub struct GetDomain {
  /// Id or name
  #[serde(alias = "id", alias = "name")]
  pub domain: String,
}

#[typeshare]
pub type GetDomainResponse = Domain;

//

/// List domains matching optional query. Response: [ListDomainsResponse].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default, Resolve)]
#[empty_traits(KomodoReadRequest)]
#[response(ListDomainsResponse)]
#[error(mogh_error::Error)]
pub struct ListDomains {
  /// Structured query to filter domains.
  #[serde(default)]
  pub query: DomainQuery,
  /// Retrieve more results by incrementing the page.
  #[serde(default)]
  pub page: U64,
  /// Limit per page. Defaults to Core config `default_pagination_limit`.
  /// `limit: 0` returns all results.
  pub limit: Option<U64>,
}

#[typeshare]
pub type ListDomainsResponse = Vec<DomainListItem>;

//

/// List full domains matching optional query. Response: [ListFullDomainsResponse].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Default, Resolve)]
#[empty_traits(KomodoReadRequest)]
#[response(ListFullDomainsResponse)]
#[error(mogh_error::Error)]
pub struct ListFullDomains {
  /// Structured query to filter domains.
  #[serde(default)]
  pub query: DomainQuery,
  #[serde(default)]
  pub page: U64,
  pub limit: Option<U64>,
}

#[typeshare]
pub type ListFullDomainsResponse = Vec<Domain>;

//

/// Compute the changes needed to make the remote DNS / tunnel match the
/// domain config. Contacts the providers, but never writes. Requires Read permission.
/// Also refreshes the cached status of the domain.
/// Response: [DomainPlan].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoReadRequest)]
#[response(GetDomainPlanResponse)]
#[error(mogh_error::Error)]
pub struct GetDomainPlan {
  /// Id or name
  pub domain: String,
}

#[typeshare]
pub type GetDomainPlanResponse = DomainPlan;

//

/// List the available domain provider kinds, their capabilities
/// and example account `params`. Response: [ListDomainProviderKindsResponse].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoReadRequest)]
#[response(ListDomainProviderKindsResponse)]
#[error(mogh_error::Error)]
pub struct ListDomainProviderKinds {}

#[typeshare]
pub type ListDomainProviderKindsResponse = Vec<DomainProviderKind>;

//

/// List domain accounts (database + core config), without tokens.
/// Admin only. Response: [ListDomainAccountsResponse].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoReadRequest)]
#[response(ListDomainAccountsResponse)]
#[error(mogh_error::Error)]
pub struct ListDomainAccounts {}

#[typeshare]
pub type ListDomainAccountsResponse = Vec<DomainAccount>;

//

/// Get a domain account by id or name, without the token.
/// Admin only. Response: [DomainAccount].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoReadRequest)]
#[response(GetDomainAccountResponse)]
#[error(mogh_error::Error)]
pub struct GetDomainAccount {
  /// Id or name
  #[serde(alias = "id", alias = "name")]
  pub account: String,
}

#[typeshare]
pub type GetDomainAccountResponse = DomainAccount;

//

/// **Admin only.** List the zones and tunnels a provider token can see
/// (read only, nothing is saved). Pass `account` to use a saved token, or
/// `kind` + `token` to preview before creating the account.
/// Response: [DomainDiscovery].
#[typeshare]
#[derive(Serialize, Deserialize, Clone, Resolve)]
#[empty_traits(KomodoReadRequest)]
#[response(DiscoverDomainAccountResponse)]
#[error(mogh_error::Error)]
pub struct DiscoverDomainAccount {
  /// Id or name of a saved account.
  pub account: Option<String>,
  /// Provider kind, used with `token` when `account` is not set.
  pub kind: Option<String>,
  /// Unsaved provider token.
  pub token: Option<String>,
}

#[typeshare]
pub type DiscoverDomainAccountResponse = DomainDiscovery;

// No derived Debug, so the token can't be logged.
impl fmt::Debug for DiscoverDomainAccount {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("DiscoverDomainAccount")
      .field("account", &self.account)
      .field("kind", &self.kind)
      .field("token", &self.token.as_ref().map(|_| "***"))
      .finish()
  }
}
