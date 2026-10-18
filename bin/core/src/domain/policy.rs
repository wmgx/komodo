//! Pure admission policies shared by recovery and fresh execution.
//! Tests here exercise permission and concurrency boundaries without provider calls.
use anyhow::{Result, bail};
use komodo_client::entities::{
  domain::DomainPlanOperation, permission::PermissionLevel,
};

pub fn required_permission(
  operation: &DomainPlanOperation,
) -> PermissionLevel {
  match operation {
    DomainPlanOperation::Reconcile => PermissionLevel::Execute,
    _ => PermissionLevel::Write,
  }
}

pub fn validate_generation(
  current: i64,
  expected: i64,
  recovering: bool,
  same_config: bool,
) -> Result<()> {
  if current < 1 || expected < 1 {
    bail!(
      "GenerationConflict: resource generations must be positive"
    );
  }
  if current == expected
    || (recovering
      && same_config
      && expected.checked_add(1) == Some(current))
  {
    return Ok(());
  }
  bail!("StalePlan: domain generation changed")
}

pub fn request_digest(
  plan_id: &str,
  expected: i64,
  method: &str,
  user_id: &str,
) -> Result<String> {
  super::digest(&(plan_id, expected, method, user_id))
}

pub fn validate_request_digest(
  saved: &str,
  requested: &str,
) -> Result<()> {
  if saved != requested {
    bail!(
      "IdempotencyConflict: request ID already used for another operation"
    );
  }
  Ok(())
}

pub fn validate_plan_instance(
  current: &str,
  saved: &str,
) -> Result<()> {
  if current.is_empty() || current != saved {
    bail!("OwnershipUnconfirmed: plan belongs to another instance");
  }
  Ok(())
}

pub fn validate_ownership_instance(
  current: &str,
  saved: &str,
  has_objects: bool,
) -> Result<()> {
  if current.is_empty()
    || (!saved.is_empty() && saved != current)
    || (saved.is_empty() && has_objects)
  {
    bail!(
      "OwnershipUnconfirmed: restored ownership requires explicit instance inheritance"
    );
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use komodo_client::entities::permission::PermissionLevelAndSpecifics;

  #[test]
  fn execute_permission_cannot_change_configuration_or_ownership() {
    let execute =
      PermissionLevelAndSpecifics::from(PermissionLevel::Execute);
    assert!(execute.fulfills(
      &required_permission(&DomainPlanOperation::Reconcile).into()
    ));
    for operation in [
      DomainPlanOperation::Create,
      DomainPlanOperation::Update,
      DomainPlanOperation::Remove,
      DomainPlanOperation::Migrate,
      DomainPlanOperation::Adopt,
      DomainPlanOperation::Abandon,
      DomainPlanOperation::FinalizeMigration,
    ] {
      assert!(
        !execute.fulfills(&required_permission(&operation).into()),
        "{operation:?}"
      );
    }
  }

  #[test]
  fn request_identity_is_bound_to_plan_generation_method_and_caller()
  {
    let original =
      request_digest("plan-a", 3, "ReconcileDomain", "alice")
        .unwrap();
    assert!(
      validate_request_digest(
        &original,
        &request_digest("plan-a", 3, "ReconcileDomain", "alice")
          .unwrap()
      )
      .is_ok()
    );
    for candidate in [
      request_digest("plan-b", 3, "ReconcileDomain", "alice"),
      request_digest("plan-a", 4, "ReconcileDomain", "alice"),
      request_digest("plan-a", 3, "RemoveDomain", "alice"),
      request_digest("plan-a", 3, "ReconcileDomain", "bob"),
    ] {
      assert!(
        validate_request_digest(&original, &candidate.unwrap())
          .is_err()
      );
    }
  }

  #[test]
  fn recovery_only_accepts_the_single_already_committed_generation() {
    assert!(validate_generation(3, 3, false, false).is_ok());
    assert!(validate_generation(4, 3, false, true).is_err());
    assert!(validate_generation(4, 3, true, false).is_err());
    assert!(validate_generation(4, 3, true, true).is_ok());
    assert!(validate_generation(5, 3, true, true).is_err());
    assert!(validate_generation(0, 0, true, true).is_err());
    assert!(validate_generation(1, i64::MAX, true, true).is_err());
  }

  #[test]
  fn backup_restore_never_implicitly_inherits_ownership() {
    assert!(validate_plan_instance("new", "old").is_err());
    assert!(validate_plan_instance("", "").is_err());
    assert!(validate_plan_instance("old", "old").is_ok());
    assert!(validate_ownership_instance("new", "old", true).is_err());
    assert!(validate_ownership_instance("new", "", true).is_err());
    assert!(validate_ownership_instance("new", "", false).is_ok());
    assert!(validate_ownership_instance("old", "old", true).is_ok());
  }

  #[test]
  fn normalized_hostnames_share_the_database_uniqueness_key() {
    use super::super::validation::hostname;
    assert_eq!(
      hostname("APP.Example.COM.").unwrap(),
      hostname("app.example.com").unwrap()
    );
    assert_eq!(
      hostname("bücher.example").unwrap(),
      hostname("xn--bcher-kva.example").unwrap()
    );
  }
}
