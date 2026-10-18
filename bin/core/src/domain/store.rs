//! Persistence of `Domain.info` (status + owned objects).

use database::{
  bson::to_bson,
  mungos::mongodb::bson::{doc, oid::ObjectId},
};
use komodo_client::entities::{
  domain::{DomainStatus, OwnedObject},
  komodo_timestamp,
};

use crate::state::db_client;

use super::error::DomainError;

/// Store the observed status.
pub async fn save_status(
  domain_id: &str,
  status: &DomainStatus,
) -> Result<(), DomainError> {
  let status = to_bson(status).map_err(DomainError::internal)?;
  update_info(
    domain_id,
    doc! { "info.status": status, "info.observed_at": komodo_timestamp() },
  )
  .await
}

/// Store the owned objects (and optionally the status).
pub async fn save_owned(
  domain_id: &str,
  owned: &[OwnedObject],
) -> Result<(), DomainError> {
  let owned = to_bson(owned).map_err(DomainError::internal)?;
  update_info(domain_id, doc! { "info.owned": owned }).await
}

async fn update_info(
  domain_id: &str,
  set: database::bson::Document,
) -> Result<(), DomainError> {
  let oid =
    ObjectId::parse_str(domain_id).map_err(DomainError::internal)?;
  db_client()
    .domains
    .update_one(doc! { "_id": oid }, doc! { "$set": set })
    .await
    .map_err(|e| DomainError::internal(format!("{e:#}")))?;
  Ok(())
}
