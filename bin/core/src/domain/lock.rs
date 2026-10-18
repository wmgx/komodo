use std::{
  collections::{BTreeSet, HashMap},
  sync::{Arc, LazyLock, Mutex},
};

use tokio::sync::OwnedMutexGuard;

type AccountLock = Arc<tokio::sync::Mutex<()>>;

static LOCKS: LazyLock<Mutex<HashMap<String, AccountLock>>> =
  LazyLock::new(Default::default);

/// Guards held for the duration of a plan / apply / cleanup.
pub struct AccountGuards(
  #[allow(dead_code)] Vec<OwnedMutexGuard<()>>,
);

/// Lock the given accounts (in name order, so concurrent callers
/// cannot deadlock). One Core process = one set of locks.
pub async fn lock_accounts<'a>(
  names: impl IntoIterator<Item = &'a str>,
) -> AccountGuards {
  let names: BTreeSet<&str> = names.into_iter().collect();
  let mut guards = Vec::with_capacity(names.len());
  for name in names {
    let lock = {
      // A poisoned map is still usable: it only holds Arcs.
      let mut map = LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
      map.entry(name.to_string()).or_default().clone()
    };
    guards.push(lock.lock_owned().await);
  }
  AccountGuards(guards)
}
