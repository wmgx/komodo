//! Apply a plan, and clean up on delete.
//!
//! There is no rollback: apply is idempotent and converges, so after a
//! failure the next plan simply shows what is left.

use komodo_client::entities::domain::{
  Change, ChangeAction, Domain, DomainLayer, DomainPlan,
  DomainReason, DomainStatus, OnDelete, OwnedObject,
};
use tokio::time::{Duration, timeout};

use crate::resource;

use super::{
  accounts,
  error::{DomainError, code},
  lock::lock_accounts,
  plan::{compute_changes, status_of},
  prepare::{
    Planned, Prepared, account_names_of, make_plan, observe,
    plan_input,
  },
  provider::{RemoteRecord, digest},
  store,
};

/// Upper bound for one whole apply.
const APPLY_TIMEOUT: Duration = Duration::from_secs(120);

pub struct ApplyOutcome {
  /// The plan which was executed (or refused).
  pub plan: DomainPlan,
  /// Changes which were written successfully.
  pub executed: Vec<Change>,
  /// Why execution stopped, if it did.
  pub error: Option<DomainReason>,
  /// Status after apply.
  pub status: DomainStatus,
}

fn refuse(
  planned: Planned,
  error: DomainError,
  key: &str,
) -> ApplyOutcome {
  error.log("apply refused");
  let reason = error.to_reason(None, Some(key));
  ApplyOutcome {
    status: DomainStatus::Error {
      reason: reason.clone(),
    },
    plan: planned.plan,
    executed: Vec::new(),
    error: Some(reason),
  }
}

/// Make the remote side match the Domain config.
/// `expected_plan_id`: refuse unless the fresh plan has this id.
pub async fn apply(
  domain: &Domain,
  expected_plan_id: Option<&str>,
  adopt_conflicts: bool,
) -> Result<ApplyOutcome, DomainError> {
  let _guards = lock_accounts(
    account_names_of(domain).iter().map(String::as_str),
  )
  .await;
  // Re-read under the lock: `owned` may have changed meanwhile.
  let domain = resource::get::<Domain>(&domain.id)
    .await
    .map_err(DomainError::internal)?;

  let outcome = timeout(
    APPLY_TIMEOUT,
    apply_locked(&domain, expected_plan_id, adopt_conflicts),
  )
  .await
  .map_err(|_| {
    DomainError::new(code::TIMEOUT, "Apply timed out").retryable()
  })??;

  if let Err(e) =
    store::save_status(&domain.id, &outcome.status).await
  {
    e.log("save domain status");
  }
  Ok(outcome)
}

async fn apply_locked(
  domain: &Domain,
  expected_plan_id: Option<&str>,
  adopt_conflicts: bool,
) -> Result<ApplyOutcome, DomainError> {
  let key = domain.config.hostname.clone();
  let planned = make_plan(domain).await;

  if let Some(reason) = planned.plan.errors.first().cloned() {
    return Ok(ApplyOutcome {
      status: DomainStatus::Error {
        reason: reason.clone(),
      },
      plan: planned.plan,
      executed: Vec::new(),
      error: Some(reason),
    });
  }
  if expected_plan_id.is_some_and(|id| id != planned.plan.plan_id) {
    return Ok(refuse(
      planned,
      DomainError::new(
        code::PLAN_CHANGED,
        "The plan changed since it was reviewed, review the new plan",
      ),
      &key,
    ));
  }
  if !adopt_conflicts
    && planned
      .plan
      .changes
      .iter()
      .any(|c| c.action == ChangeAction::Conflict)
  {
    return Ok(refuse(
      planned,
      DomainError::new(
        code::FOREIGN_CONFLICT,
        "Remote objects exist which Komodo does not own. Review the plan and set adopt_conflicts to overwrite them",
      ),
      &key,
    ));
  }

  let (Some(prepared), Some(observed)) =
    (planned.prepared.as_ref(), planned.observed.as_ref())
  else {
    return Err(DomainError::internal("plan without context"));
  };

  let mut owned = prepared.owned.clone();
  let mut executed = Vec::new();
  let mut failure: Option<DomainReason> = None;

  for change in &planned.plan.changes {
    if change.action == ChangeAction::Noop {
      continue;
    }
    let result =
      execute_change(change, prepared, &observed.records, &mut owned)
        .await;
    match result {
      Ok(()) => {
        executed.push(change.clone());
        if let Err(e) = store::save_owned(&domain.id, &owned).await {
          e.log("save owned objects");
          failure = Some(e.to_reason(Some(change.layer), Some(&key)));
          break;
        }
      }
      Err(e) => {
        e.log(&format!(
          "apply {:?} {:?}",
          change.action, change.object
        ));
        failure = Some(
          e.to_reason(Some(change.layer), Some(&change.object.key)),
        );
        break;
      }
    }
  }

  let status = match &failure {
    Some(reason) => DomainStatus::Error {
      reason: reason.clone(),
    },
    None => observe_status(prepared, owned).await,
  };
  Ok(ApplyOutcome {
    plan: planned.plan,
    executed,
    error: failure,
    status,
  })
}

/// Observe again after a successful apply.
async fn observe_status(
  prepared: &Prepared,
  owned: Vec<OwnedObject>,
) -> DomainStatus {
  let observed = match observe(prepared).await {
    Ok(o) => o,
    Err(e) => {
      e.error.log("observe after apply");
      return DomainStatus::Error {
        reason: e.to_reason(&prepared.hostname),
      };
    }
  };
  let mut input = plan_input(prepared, &observed);
  input.owned = &owned;
  status_of(&compute_changes(&input), &[])
}

fn replace_owned(owned: &mut Vec<OwnedObject>, new: OwnedObject) {
  owned.retain(|o| {
    !(o.layer == new.layer
      && o.key == new.key
      && o.account == new.account)
  });
  owned.push(new);
}

async fn execute_change(
  change: &Change,
  prepared: &Prepared,
  records: &[(RemoteRecord, bool)],
  owned: &mut Vec<OwnedObject>,
) -> Result<(), DomainError> {
  let hostname = prepared.hostname.as_str();
  match (change.action, change.layer) {
    (ChangeAction::Delete, layer) => {
      let target = owned
        .iter()
        .find(|o| {
          o.layer == layer
            && o.key == change.object.key
            && change
              .object
              .remote_id
              .as_ref()
              .is_none_or(|id| *id == o.remote_id)
        })
        .cloned()
        .ok_or_else(|| {
          DomainError::internal("owned object vanished")
        })?;
      delete_owned(&target, &prepared.owner).await?;
      owned.retain(|o| o != &target);
    }
    (
      ChangeAction::Create
      | ChangeAction::Update
      | ChangeAction::Conflict,
      DomainLayer::Ingress,
    ) => {
      let ingress = prepared
        .ingress
        .as_ref()
        .ok_or_else(|| DomainError::internal("missing ingress"))?;
      let rule = ingress
        .provider
        .put(&ingress.target, hostname, &ingress.service)
        .await?;
      replace_owned(
        owned,
        OwnedObject {
          layer: DomainLayer::Ingress,
          account: ingress.account.clone(),
          key: hostname.to_string(),
          remote_id: String::new(),
          scope: ingress.target.id.clone(),
          applied_digest: digest(&rule.rule),
        },
      );
    }
    (
      ChangeAction::Create
      | ChangeAction::Update
      | ChangeAction::Conflict,
      DomainLayer::Dns,
    ) => {
      let dns = prepared
        .dns
        .as_ref()
        .ok_or_else(|| DomainError::internal("missing dns"))?;
      let existing = match change.action {
        ChangeAction::Create => None,
        _ => {
          let id = change.object.remote_id.as_deref();
          let found = records
            .iter()
            .map(|(r, _)| r)
            .find(|r| Some(r.remote_id.as_str()) == id);
          if found.is_none() {
            return Err(DomainError::internal(
              "record to update not found in observation",
            ));
          }
          found
        }
      };
      if change.action == ChangeAction::Conflict && records.len() != 1
      {
        return Err(DomainError::new(
          code::FOREIGN_CONFLICT,
          "Cannot adopt: more than one record exists for this hostname",
        ));
      }
      let record = dns
        .provider
        .upsert(hostname, &dns.spec, &prepared.owner, existing)
        .await?;
      replace_owned(
        owned,
        OwnedObject {
          layer: DomainLayer::Dns,
          account: dns.account.clone(),
          key: hostname.to_string(),
          remote_id: record.remote_id,
          scope: record.scope,
          applied_digest: digest(&record.canonical),
        },
      );
    }
    (ChangeAction::Noop, _) => {}
  }
  Ok(())
}

async fn delete_owned(
  owned: &OwnedObject,
  owner: &super::provider::Owner,
) -> Result<(), DomainError> {
  let providers =
    accounts::require(&owned.account).await?.providers()?;
  match owned.layer {
    DomainLayer::Dns => {
      let dns = providers.dns.ok_or_else(|| {
        DomainError::new(
          code::ACCOUNT_KIND_MISMATCH,
          "Account cannot manage DNS",
        )
      })?;
      dns.delete(owned, owner).await
    }
    DomainLayer::Ingress => {
      let ingress = providers.ingress.ok_or_else(|| {
        DomainError::new(
          code::ACCOUNT_KIND_MISMATCH,
          "Account cannot provide ingress",
        )
      })?;
      ingress.remove(owned).await
    }
  }
}

/// Called before a Domain is deleted. With `OnDelete::Cleanup`, removes
/// everything Komodo created; any failure aborts the delete.
/// Returns human readable log lines.
pub async fn cleanup(
  domain: &Domain,
) -> Result<Vec<String>, DomainError> {
  if domain.config.on_delete == OnDelete::Retain
    || domain.info.owned.is_empty()
  {
    return Ok(Vec::new());
  }
  let _guards = lock_accounts(
    account_names_of(domain).iter().map(String::as_str),
  )
  .await;
  let domain = resource::get::<Domain>(&domain.id)
    .await
    .map_err(DomainError::internal)?;
  let owner = super::provider::Owner {
    id: domain.id.clone(),
  };

  let mut remaining = domain.info.owned.clone();
  let mut lines = Vec::new();
  // DNS first, then ingress rules.
  let mut order: Vec<OwnedObject> = remaining.clone();
  order.sort_by_key(|o| match o.layer {
    DomainLayer::Dns => 0,
    DomainLayer::Ingress => 1,
  });
  for object in order {
    let result =
      timeout(APPLY_TIMEOUT, delete_owned(&object, &owner))
        .await
        .unwrap_or_else(|_| {
          Err(
            DomainError::new(code::TIMEOUT, "Cleanup timed out")
              .retryable(),
          )
        });
    if let Err(e) = result {
      e.log(&format!("cleanup of {} {:?}", object.key, object.layer));
      // Persist progress so a retry continues where this stopped.
      if let Err(save) =
        store::save_owned(&domain.id, &remaining).await
      {
        save.log("save owned objects after failed cleanup");
      }
      return Err(DomainError {
        message: format!(
          "Failed to clean up {:?} object for {}: {}. Fix the cause, or set on_delete to Retain to delete without touching the remote side",
          object.layer, object.key, e.message
        ),
        ..e
      });
    }
    remaining.retain(|o| o != &object);
    lines.push(format!(
      "Removed {:?} object for {}",
      object.layer, object.key
    ));
  }
  store::save_owned(&domain.id, &remaining).await?;
  Ok(lines)
}
