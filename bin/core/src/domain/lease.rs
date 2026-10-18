//! Durable single-writer lease. A replacement waits longer than any bounded
//! adapter request before acquiring a fencing epoch.
use crate::state::db_client;
use komodo_client::entities::komodo_timestamp;
use std::sync::OnceLock;
use uuid::Uuid;

const LEASE_MS: i64 = 120_000;
fn owner() -> &'static str {
  static OWNER: OnceLock<String> = OnceLock::new();
  OWNER.get_or_init(|| Uuid::new_v4().to_string())
}

pub struct Lease {
  key: String,
  pub fence: i64,
}
impl Lease {
  pub async fn acquire(key: &str) -> anyhow::Result<Self> {
    let fence = database::domain::acquire_lease(
      &db_client().domain_leases,
      key,
      owner(),
      komodo_timestamp(),
      LEASE_MS,
    )
    .await?;
    Ok(Self {
      key: key.into(),
      fence,
    })
  }
  /// Renew immediately before every external write. Adapter timeouts must stay below LEASE_MS.
  pub async fn renew(&self) -> anyhow::Result<()> {
    database::domain::renew_lease(
      &db_client().domain_leases,
      &self.key,
      owner(),
      self.fence,
      komodo_timestamp(),
      LEASE_MS,
    )
    .await
  }
  pub async fn release(self) -> anyhow::Result<()> {
    database::domain::release_lease(
      &db_client().domain_leases,
      &self.key,
      owner(),
      self.fence,
    )
    .await
  }
}

/// Acquire once during startup. The process exits on lost ownership so a
/// second writer cannot continue using an old in-memory controller state.
pub async fn start_controller() -> anyhow::Result<()> {
  let lease = Lease::acquire("controller").await?;
  tokio::spawn(async move {
    let mut interval =
      tokio::time::interval(std::time::Duration::from_secs(20));
    loop {
      interval.tick().await;
      if lease.renew().await.is_err() {
        error!("Domain controller lease lost; stopping Core");
        std::process::exit(1);
      }
    }
  });
  Ok(())
}
