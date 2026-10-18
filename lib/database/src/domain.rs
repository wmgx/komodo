//! Atomic MongoDB lease operations used by the domain single-writer controller.
use crate::{
  bson::{Document, doc},
  mungos::mongodb::{Collection, options::ReturnDocument},
};
use anyhow::{Context, Result, bail};

pub async fn acquire_lease(
  coll: &Collection<Document>,
  key: &str,
  owner: &str,
  now: i64,
  duration: i64,
) -> Result<i64> {
  let expires = now
    .checked_add(duration)
    .context("LeaseInvalid: duration overflow")?;
  if duration <= 0 || owner.is_empty() {
    bail!("LeaseInvalid: positive duration and owner required")
  }
  // An existing _id is expected; propagate actual database failures.
  if let Err(error) = coll
    .insert_one(
      doc! {"_id":key,"owner":"","expires":0_i64,"fence":0_i64},
    )
    .await
  {
    if !matches!(error.kind.as_ref(), crate::mungos::mongodb::error::ErrorKind::Write(crate::mungos::mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
    {
      return Err(error.into());
    }
  }
  let state=coll.find_one_and_update(doc! {"_id":key,"expires":{"$lt":now}},doc! {"$set":{"owner":owner,"expires":expires},"$inc":{"fence":1_i64}}).return_document(ReturnDocument::After).await?.context("DomainBusy: controller or resource lease is held")?;
  Ok(state.get_i64("fence")?)
}

pub async fn renew_lease(
  coll: &Collection<Document>,
  key: &str,
  owner: &str,
  fence: i64,
  now: i64,
  duration: i64,
) -> Result<()> {
  let expires = now
    .checked_add(duration)
    .context("LeaseInvalid: duration overflow")?;
  if duration <= 0 {
    bail!("LeaseInvalid: positive duration required")
  }
  let result=coll.update_one(doc! {"_id":key,"owner":owner,"fence":fence,"expires":{"$gt":now}},doc! {"$set":{"expires":expires}}).await?;
  if result.matched_count != 1 {
    bail!("LeaseLost: stop starting external writes")
  }
  Ok(())
}

pub async fn release_lease(
  coll: &Collection<Document>,
  key: &str,
  owner: &str,
  fence: i64,
) -> Result<()> {
  coll
    .update_one(
      doc! {"_id":key,"owner":owner,"fence":fence},
      doc! {"$set":{"expires":0_i64}},
    )
    .await?;
  Ok(())
}
