//! Domain provider accounts. Database accounts take priority over
//! accounts declared in the Core config.

use std::str::FromStr;

use database::mungos::{
  find::find_collect,
  mongodb::{
    bson::{doc, oid::ObjectId},
    options::FindOptions,
  },
};
use komodo_client::entities::{
  JsonValue,
  domain::DomainLayer,
  domain_account::{
    DomainAccount, DomainAccountDoc, DomainAccountSource,
  },
};

use crate::{config::core_config, state::db_client};

use super::{
  error::{DomainError, code},
  provider::{Credentials, ProviderFactory, ProviderSet, factory},
};

/// An account with its token, for internal use only.
pub struct ResolvedAccount {
  pub id: String,
  pub name: String,
  pub kind: String,
  pub token: String,
  pub params: JsonValue,
  pub description: String,
  pub source: DomainAccountSource,
}

fn capabilities(kind: &str) -> Vec<DomainLayer> {
  factory(kind)
    .map(|f| f.capabilities.to_vec())
    .unwrap_or_default()
}

impl ResolvedAccount {
  /// Redacted view. Never contains the token.
  pub fn view(&self) -> DomainAccount {
    DomainAccount {
      id: self.id.clone(),
      name: self.name.clone(),
      kind: self.kind.clone(),
      params: self.params.clone(),
      description: self.description.clone(),
      token_set: !self.token.is_empty(),
      source: self.source,
      capabilities: capabilities(&self.kind),
    }
  }

  pub fn factory(
    &self,
  ) -> Result<&'static ProviderFactory, DomainError> {
    factory(&self.kind).ok_or_else(|| {
      DomainError::new(
        code::ACCOUNT_KIND_MISMATCH,
        format!(
          "Account '{}' has unknown provider kind '{}'",
          self.name, self.kind
        ),
      )
    })
  }

  /// Build the provider implementations. No network.
  pub fn providers(&self) -> Result<ProviderSet, DomainError> {
    (self.factory()?.build)(Credentials {
      name: self.name.clone(),
      token: self.token.clone(),
      params: self.params.clone(),
    })
  }
}

impl From<DomainAccountDoc> for ResolvedAccount {
  fn from(doc: DomainAccountDoc) -> Self {
    Self {
      id: doc.id,
      name: doc.name,
      kind: doc.kind,
      token: doc.token,
      params: doc.params,
      description: doc.description,
      source: DomainAccountSource::Database,
    }
  }
}

/// Find by exact name (what Domains reference). DB first, then config.
pub async fn find_by_name(
  name: &str,
) -> Result<Option<ResolvedAccount>, DomainError> {
  let from_db = db_client()
    .domain_accounts
    .find_one(doc! { "name": name })
    .await
    .map_err(|e| DomainError::internal(format!("{e:#}")))?;
  if let Some(doc) = from_db {
    return Ok(Some(doc.into()));
  }
  Ok(
    core_config()
      .domain_accounts
      .iter()
      .find(|a| a.name == name)
      .map(|a| ResolvedAccount {
        id: String::new(),
        name: a.name.clone(),
        kind: a.kind.clone(),
        token: a.token.clone(),
        params: a.params.clone(),
        description: a.description.clone(),
        source: DomainAccountSource::Config,
      }),
  )
}

/// Account which a Domain references, or `AccountNotFound`.
pub async fn require(
  name: &str,
) -> Result<ResolvedAccount, DomainError> {
  find_by_name(name).await?.ok_or_else(|| {
    DomainError::new(
      code::ACCOUNT_NOT_FOUND,
      format!("Domain account '{name}' does not exist"),
    )
  })
}

/// Find by id or name (for the account API).
pub async fn find_by_id_or_name(
  id_or_name: &str,
) -> Result<Option<ResolvedAccount>, DomainError> {
  if let Ok(oid) = ObjectId::from_str(id_or_name) {
    let found = db_client()
      .domain_accounts
      .find_one(doc! { "_id": oid })
      .await
      .map_err(|e| DomainError::internal(format!("{e:#}")))?;
    if let Some(found) = found {
      return Ok(Some(found.into()));
    }
  }
  find_by_name(id_or_name).await
}

/// All accounts: database first, then config ones not shadowed by name.
pub async fn list_all() -> Result<Vec<ResolvedAccount>, DomainError> {
  let mut accounts: Vec<ResolvedAccount> = find_collect(
    &db_client().domain_accounts,
    None,
    FindOptions::builder().sort(doc! { "name": 1 }).build(),
  )
  .await
  .map_err(|e| DomainError::internal(format!("{e:#}")))?
  .into_iter()
  .map(Into::into)
  .collect();
  for config in &core_config().domain_accounts {
    if accounts.iter().all(|a| a.name != config.name) {
      accounts.push(ResolvedAccount {
        id: String::new(),
        name: config.name.clone(),
        kind: config.kind.clone(),
        token: config.token.clone(),
        params: config.params.clone(),
        description: config.description.clone(),
        source: DomainAccountSource::Config,
      });
    }
  }
  Ok(accounts)
}
