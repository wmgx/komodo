use komodo_client::{
  api::execute::ApplyDomain,
  entities::{domain::Domain, permission::PermissionLevel},
};
use mogh_resolver::Resolve;

use crate::{
  domain::apply::apply, helpers::update::update_update,
  permission::get_check_permissions,
};

use super::ExecuteArgs;

impl Resolve<ExecuteArgs> for ApplyDomain {
  #[instrument(
    "ApplyDomain",
    skip_all,
    fields(
      task_id = task_id.to_string(),
      operator = user.id,
      update_id = update.id,
      request = format!("{self:?}"),
    )
  )]
  async fn resolve(
    self,
    ExecuteArgs {
      user,
      update,
      task_id,
    }: &ExecuteArgs,
  ) -> Result<Self::Response, Self::Error> {
    let domain = get_check_permissions::<Domain>(
      &self.domain,
      user,
      PermissionLevel::Execute.into(),
    )
    .await?;

    let mut update = update.clone();
    update_update(update.clone()).await?;

    match apply(
      &domain,
      self.plan_id.as_deref(),
      self.adopt_conflicts,
    )
    .await
    {
      Ok(outcome) => {
        let executed =
          serde_json::to_string_pretty(&outcome.executed)
            .unwrap_or_default();
        let plan = serde_json::to_string_pretty(&outcome.plan)
          .unwrap_or_default();
        update.push_simple_log("Plan", plan);
        match outcome.error {
          None => update.push_simple_log("Applied changes", executed),
          Some(reason) => {
            update.push_simple_log("Applied changes", executed);
            update.push_error_log(
              "Apply failed",
              serde_json::to_string_pretty(&reason)
                .unwrap_or_default(),
            );
          }
        }
      }
      Err(e) => {
        e.log("apply domain");
        update.push_error_log(
          "Apply failed",
          format!("{}: {}", e.code, e.message),
        );
      }
    }

    update.finalize();
    update_update(update.clone()).await?;
    Ok(update)
  }
}
