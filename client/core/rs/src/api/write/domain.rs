use std::fmt;

use mogh_resolver::Resolve;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::entities::{
  JsonValue, ResourceTarget,
  domain::{
    _PartialDomainConfig, Domain, DomainMode, ServiceDomainResult,
  },
  domain_account::DomainAccount,
  update::Update,
};

use super::KomodoWriteRequest;

//

/// Create a domain. Response: [Domain].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(Domain)]
#[error(mogh_error::Error)]
pub struct CreateDomain {
  /// The name given to newly created domain.
  pub name: String,
  /// Optional partial config to initialize the domain with.
  #[serde(default)]
  pub config: _PartialDomainConfig,
}

//

/// **Admin only.** One step: give a Deployment / Stack service a public
/// hostname. Validates, derives account / zone / tunnel / record from the
/// target's Server, creates the Domain and applies it.
/// If remote objects exist which Komodo does not own and `adopt_conflicts`
/// is false, nothing is written and they are returned in `conflicts`.
/// Response: [AddServiceDomainResponse].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(AddServiceDomainResponse)]
#[error(mogh_error::Error)]
pub struct AddServiceDomain {
  /// Deployment or Stack (id or name inside the target).
  pub target: ResourceTarget,
  /// Service name, required for Stacks.
  #[serde(default)]
  pub service: String,
  /// Full hostname, eg. `app.example.com`.
  pub hostname: String,
  /// Published port. Detected from the Deployment config when 0.
  #[serde(default)]
  pub port: u16,
  #[serde(default)]
  pub mode: DomainMode,
  #[serde(default)]
  pub adopt_conflicts: bool,
}

#[typeshare]
pub type AddServiceDomainResponse = ServiceDomainResult;

//

/// Creates a new domain with given `name` and the configuration
/// of the domain at the given `id`. Response: [Domain].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(Domain)]
#[error(mogh_error::Error)]
pub struct CopyDomain {
  /// The name of the new domain.
  pub name: String,
  /// The id of the domain to copy.
  pub id: String,
}

//

/// Deletes the domain at the given id, and returns the deleted domain.
/// With `on_delete: Cleanup` (default) remote objects created by Komodo are
/// removed first, and the delete is refused if that fails.
/// Set `on_delete: Retain` on the domain config to keep remote objects.
/// Response: [Domain]
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(Domain)]
#[error(mogh_error::Error)]
pub struct DeleteDomain {
  /// The id or name of the domain to delete.
  pub id: String,
}

//

/// Update the domain at the given id, and return the updated domain.
/// Only fields set in the partial config are changed. Response: [Domain].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(Domain)]
#[error(mogh_error::Error)]
pub struct UpdateDomain {
  /// The id of the domain to update.
  pub id: String,
  /// The partial config update to apply.
  pub config: _PartialDomainConfig,
}

//

/// Rename the Domain at id to the given name. Response: [Update].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(Update)]
#[error(mogh_error::Error)]
pub struct RenameDomain {
  /// The id or name of the Domain to rename.
  pub id: String,
  /// The new name.
  pub name: String,
}

//

/// **Admin only.** Create a domain provider account. The token is stored
/// in plain text on the database (same as git provider tokens) and is
/// never returned. Response: [DomainAccount].
#[typeshare]
#[derive(Serialize, Deserialize, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(CreateDomainAccountResponse)]
#[error(mogh_error::Error)]
pub struct CreateDomainAccount {
  /// Globally unique. Domains reference the account by this name.
  pub name: String,
  /// Provider kind, eg. `cloudflare`. See ListDomainProviderKinds.
  pub kind: String,
  /// The provider API token.
  pub token: String,
  /// Non-secret provider params. See ListDomainProviderKinds for an example.
  #[serde(default)]
  pub params: JsonValue,
  #[serde(default)]
  pub description: String,
}

#[typeshare]
pub type CreateDomainAccountResponse = DomainAccount;

//

/// **Admin only.** Update a database domain account. Name and kind cannot
/// change. Empty / missing `token` keeps the current token.
/// Response: [DomainAccount].
#[typeshare]
#[derive(Serialize, Deserialize, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(UpdateDomainAccountResponse)]
#[error(mogh_error::Error)]
pub struct UpdateDomainAccount {
  /// Id or name
  pub id: String,
  pub token: Option<String>,
  pub params: Option<JsonValue>,
  pub description: Option<String>,
}

#[typeshare]
pub type UpdateDomainAccountResponse = DomainAccount;

//

/// **Admin only.** Delete a database domain account.
/// Refused while any Domain references it. Response: [DomainAccount].
#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Resolve)]
#[empty_traits(KomodoWriteRequest)]
#[response(DeleteDomainAccountResponse)]
#[error(mogh_error::Error)]
pub struct DeleteDomainAccount {
  /// Id or name
  pub id: String,
}

#[typeshare]
pub type DeleteDomainAccountResponse = DomainAccount;

// Requests carrying a token don't derive Debug, so the token can't be logged.

impl fmt::Debug for CreateDomainAccount {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("CreateDomainAccount")
      .field("name", &self.name)
      .field("kind", &self.kind)
      .field("token", &"***")
      .finish()
  }
}

impl fmt::Debug for UpdateDomainAccount {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("UpdateDomainAccount")
      .field("id", &self.id)
      .field("token", &self.token.as_ref().map(|_| "***"))
      .finish()
  }
}
