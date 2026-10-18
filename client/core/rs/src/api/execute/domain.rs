use super::KomodoExecuteRequest;
use crate::entities::{I64, update::Update};
use mogh_resolver::Resolve;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/ReconcileDomain",request_body(content=ReconcileDomain),responses((status=200,body=Update)))]
pub fn reconcile_domain() {}
#[typeshare]
#[derive(
  Debug,
  Clone,
  Serialize,
  Deserialize,
  Resolve,
  PartialEq,
  clap::Parser,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(mogh_error::Error)]
pub struct ReconcileDomain {
  pub domain: String,
  #[clap(long, default_value = "")]
  pub plan_id: String,
  #[clap(long, default_value_t = 0)]
  pub expected_generation: I64,
  #[clap(long, default_value = "")]
  pub request_id: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/RemoveDomain",request_body(content=RemoveDomain),responses((status=200,body=Update)))]
pub fn remove_domain() {}
#[typeshare]
#[derive(
  Debug,
  Clone,
  Serialize,
  Deserialize,
  Resolve,
  PartialEq,
  clap::Parser,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(mogh_error::Error)]
pub struct RemoveDomain {
  pub domain: String,
  #[clap(long, default_value = "")]
  pub plan_id: String,
  #[clap(long, default_value_t = 0)]
  pub expected_generation: I64,
  #[clap(long, default_value = "")]
  pub request_id: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/MigrateDomain",request_body(content=MigrateDomain),responses((status=200,body=Update)))]
pub fn migrate_domain() {}
#[typeshare]
#[derive(
  Debug,
  Clone,
  Serialize,
  Deserialize,
  Resolve,
  PartialEq,
  clap::Parser,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(mogh_error::Error)]
pub struct MigrateDomain {
  pub domain: String,
  #[clap(long, default_value = "")]
  pub plan_id: String,
  #[clap(long, default_value_t = 0)]
  pub expected_generation: I64,
  #[clap(long, default_value = "")]
  pub request_id: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/FinalizeDomainMigration",request_body(content=FinalizeDomainMigration),responses((status=200,body=Update)))]
pub fn finalize_domain_migration() {}
#[typeshare]
#[derive(
  Debug,
  Clone,
  Serialize,
  Deserialize,
  Resolve,
  PartialEq,
  clap::Parser,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(mogh_error::Error)]
pub struct FinalizeDomainMigration {
  pub domain: String,
  #[clap(long, default_value = "")]
  pub plan_id: String,
  #[clap(long, default_value_t = 0)]
  pub expected_generation: I64,
  #[clap(long, default_value = "")]
  pub request_id: String,
}
