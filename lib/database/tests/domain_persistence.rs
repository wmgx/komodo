//! Requires a dedicated disposable MongoDB. Run explicitly with:
//! KOMODO_TEST_MONGO_URI=mongodb://127.0.0.1:27028 cargo test -p database --test domain_persistence -- --ignored
use anyhow::{Context, Result};
use database::{
  bson::{doc, oid::ObjectId},
  mungos::mongodb,
};
use komodo_client::entities::domain::Domain;
use std::future::IntoFuture;

#[tokio::test]
#[ignore = "requires an explicitly configured disposable MongoDB"]
async fn domain_unique_indexes_and_generation_compare_and_swap()
-> Result<()> {
  let uri = std::env::var("KOMODO_TEST_MONGO_URI").context(
    "KOMODO_TEST_MONGO_URI must point to a disposable test MongoDB",
  )?;
  let mongo = mongodb::Client::with_uri_str(uri).await?;
  let db = mongo.database(&format!(
    "komodo_domain_test_{}",
    ObjectId::new().to_hex()
  ));
  let result = async {
    let client = database::Client::from_database(db.clone()).await?;
    let mut first = Domain { name: "first".into(), ..Default::default() };
    first.config.hostname = "app.example.com".into();
    first.info.generation = 1;
    let id = client.domains.insert_one(&first).await?.inserted_id;
    let mut duplicate = first.clone();
    duplicate.name = "different-name".into();
    assert!(client.domains.insert_one(duplicate).await.is_err(), "hostname uniqueness must be enforced by MongoDB, independently of API races");

    client.domain_operations.insert_one(doc! {"request_id":"ci-retry", "digest":"request-a"}).await?;
    assert!(client.domain_operations.insert_one(doc! {"request_id":"ci-retry", "digest":"request-b"}).await.is_err(), "duplicate idempotency keys must not create a second operation");
    let saved = client.domain_operations.find_one(doc! {"request_id":"ci-retry"}).await?.context("saved operation missing")?;
    assert_eq!(saved.get_str("digest")?, "request-a");

    let filter = doc! {"_id": &id, "info.generation":1_i64};
    let change = doc! {"$inc":{"info.generation":1_i64}};
    let (left,right) = tokio::join!(
      client.domains.update_one(filter.clone(),change.clone()).into_future(),
      client.domains.update_one(filter,change).into_future()
    );
    assert_eq!(left?.matched_count + right?.matched_count,1,"only one competing generation update may win");
    let current = client.domains.find_one(doc! {"_id":id}).await?.context("domain missing")?;
    assert_eq!(current.info.generation,2);

    // Exercise the actual controller lease helper against MongoDB, including concurrent admission.
    let (left,right)=tokio::join!(
      database::domain::acquire_lease(&client.domain_leases,"domain:binding","worker-a",100,120),
      database::domain::acquire_lease(&client.domain_leases,"domain:binding","worker-b",100,120)
    );
    assert_eq!(usize::from(left.is_ok())+usize::from(right.is_ok()),1);
    let (winner,first_fence)=if let Ok(fence)=left {("worker-a",fence)} else {("worker-b",right?)};
    assert_eq!(first_fence,1);
    assert!(database::domain::acquire_lease(&client.domain_leases,"domain:binding","replacement",219,120).await.is_err());
    let new_fence=database::domain::acquire_lease(&client.domain_leases,"domain:binding","replacement",221,120).await?;
    assert_eq!(new_fence,2);
    assert!(database::domain::renew_lease(&client.domain_leases,"domain:binding",winner,first_fence,222,120).await.is_err());
    database::domain::release_lease(&client.domain_leases,"domain:binding",winner,first_fence).await?;
    assert!(database::domain::acquire_lease(&client.domain_leases,"domain:binding","third",223,120).await.is_err(),"stale release must not invalidate the current lease");
    database::domain::renew_lease(&client.domain_leases,"domain:binding","replacement",new_fence,224,120).await?;
    anyhow::Ok(())
  }.await;
  db.drop().await?;
  result
}
