use clap::Parser;
use mogh_resolver::Resolve;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use crate::entities::update::Update;

use super::KomodoExecuteRequest;

//

/// Make the remote DNS record and tunnel ingress rule match the domain config.
/// Requires Execute permission. The log of the Update contains the executed
/// changes as JSON. Response: [Update]
#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, PartialEq, Resolve, Parser,
)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[empty_traits(KomodoExecuteRequest)]
#[response(Update)]
#[error(mogh_error::Error)]
pub struct ApplyDomain {
  /// Id or name
  pub domain: String,
  /// If provided, apply fails with `PlanChanged` when the freshly computed
  /// plan differs from the one that was reviewed.
  #[serde(default)]
  #[arg(long)]
  pub plan_id: Option<String>,
  /// Overwrite existing remote objects which Komodo does not own.
  #[serde(default)]
  #[arg(long, default_value_t = false)]
  pub adopt_conflicts: bool,
}
