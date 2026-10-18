use super::ExecuteArgs;
use komodo_client::{api::execute::*, entities::update::Update};
use mogh_resolver::Resolve;
macro_rules! execute {
  ($request:ty,$kind:literal) => {
    impl Resolve<ExecuteArgs> for $request {
      async fn resolve(
        self,
        ExecuteArgs { user, .. }: &ExecuteArgs,
      ) -> mogh_error::Result<Update> {
        Ok(
          crate::domain::accept(
            self.domain,
            self.plan_id,
            self.expected_generation,
            self.request_id,
            $kind,
            user.clone(),
          )
          .await?,
        )
      }
    }
  };
}
execute!(ReconcileDomain, "ReconcileDomain");
execute!(RemoveDomain, "RemoveDomain");
execute!(MigrateDomain, "MigrateDomain");
execute!(FinalizeDomainMigration, "FinalizeDomainMigration");
