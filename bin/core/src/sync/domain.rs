//! ResourceSync uses the same public handlers and the initiating user's authority.
//! Connections absent from a file are deliberately retained.
use super::replace_ids::ReplaceIds;
use crate::{
  api::{
    execute::{ExecuteArgs, ExecuteRequest},
    read::ReadArgs,
    write::WriteArgs,
  },
  helpers::update::init_execution_update,
  state::db_client,
};
use anyhow::Context;
use database::bson::doc;
use komodo_client::{
  api::{execute::*, read::*, write::*},
  entities::{
    domain::*, toml::ResourceToml, update::Log, user::User,
  },
};
use mogh_resolver::Resolve;

pub struct DomainSyncResources {
  pub domains: Vec<ResourceToml<PartialDomainConfig>>,
  pub dns_providers: Vec<ResourceToml<PartialDnsProviderConfig>>,
  pub ingresses: Vec<ResourceToml<PartialIngressConfig>>,
  pub remove: Vec<String>,
}

pub async fn save(
  resources: &DomainSyncResources,
  user: &User,
  logs: &mut Vec<Log>,
) -> bool {
  let args = WriteArgs { user: user.clone() };
  let result: anyhow::Result<()> = async {
    for item in &resources.dns_providers {
      let config: DnsProviderConfig = item.config.clone().into();
      let existing = db_client()
        .dns_providers
        .find_one(doc! { "name": &item.name })
        .await?;
      let saved = if let Some(mut existing) = existing {
        DnsProvider::replace_ids(&mut existing.config);
        if existing.config != config {
          UpdateDnsProvider {
            id: existing.id,
            config,
            expected_generation: existing.info.generation,
          }
          .resolve(&args)
          .await
          .map_err(|e| e.error)?
        } else {
          existing
        }
      } else {
        CreateDnsProvider {
          name: item.name.clone(),
          config,
        }
        .resolve(&args)
        .await
        .map_err(|e| e.error)?
      };
      // Metadata edits use the same caller, never the privileged synthetic sync user.
      crate::resource::update_meta::<DnsProvider>(
        &saved.id,
        crate::resource::ResourceMetaUpdate {
          description: Some(item.description.clone()),
          template: Some(item.template),
          tags: Some(item.tags.clone()),
        },
        &args,
      )
      .await?;
    }
    for item in &resources.ingresses {
      let config: IngressConfig = item.config.clone().into();
      let existing = db_client()
        .ingresses
        .find_one(doc! { "name": &item.name })
        .await?;
      let saved = if let Some(mut existing) = existing {
        Ingress::replace_ids(&mut existing.config);
        if existing.config != config {
          UpdateIngress {
            id: existing.id,
            config,
            expected_generation: existing.info.generation,
          }
          .resolve(&args)
          .await
          .map_err(|e| e.error)?
        } else {
          existing
        }
      } else {
        CreateIngress {
          name: item.name.clone(),
          config,
        }
        .resolve(&args)
        .await
        .map_err(|e| e.error)?
      };
      // Metadata edits use the same caller, never the privileged synthetic sync user.
      crate::resource::update_meta::<Ingress>(
        &saved.id,
        crate::resource::ResourceMetaUpdate {
          description: Some(item.description.clone()),
          template: Some(item.template),
          tags: Some(item.tags.clone()),
        },
        &args,
      )
      .await?;
    }
    for item in &resources.domains {
      let config: DomainConfig = item.config.clone().into();
      let existing = db_client()
        .domains
        .find_one(doc! { "name": &item.name })
        .await?;
      let saved = if let Some(mut existing) = existing {
        Domain::replace_ids(&mut existing.config);
        // Compare canonical values so IDNA/trailing dots and URL serialization
        // do not create a new generation on every identical sync.
        let mut comparison = config.clone();
        comparison.hostname =
          crate::domain::validation::hostname(&comparison.hostname)?;
        if let DomainTarget::Url { url } = &mut comparison.target {
          *url = url::Url::parse(url)?.to_string();
        }
        if existing.config != comparison {
          UpdateDomain {
            id: existing.id,
            config,
            expected_generation: existing.info.generation,
          }
          .resolve(&args)
          .await
          .map_err(|e| e.error)?
        } else {
          existing
        }
      } else {
        CreateDomain {
          name: item.name.clone(),
          config,
        }
        .resolve(&args)
        .await
        .map_err(|e| e.error)?
      };
      // Metadata edits use the same caller, never the privileged synthetic sync user.
      crate::resource::update_meta::<Domain>(
        &saved.id,
        crate::resource::ResourceMetaUpdate {
          description: Some(item.description.clone()),
          template: Some(item.template),
          tags: Some(item.tags.clone()),
        },
        &args,
      )
      .await?;
    }
    Ok(())
  }
  .await;
  match result {
    Ok(()) => {
      logs.push(Log::simple("Domain configuration", "Saved desired configuration; external changes require reconciliation.".into()));
      true
    }
    Err(_) => {
      logs.push(Log::error("Domain configuration", "Domain configuration rejected; inspect the resource permissions, generation and connection constraints.".into()));
      false
    }
  }
}

pub async fn deploy(
  resources: &DomainSyncResources,
  user: &User,
  logs: &mut Vec<Log>,
) {
  for item in resources.domains.iter().filter(|item| item.deploy) {
    let result = execute(
      DomainChangeIntent::Reconcile {
        domain: item.name.clone(),
      },
      user,
    )
    .await;
    logs.push(match result {
      Ok(()) => Log::simple("Reconcile domain", format!("Reconciled {}; check DomainStatus for observed conditions.", item.name)),
      Err(_) => Log::error("Reconcile domain", format!("Could not reconcile {}; operation remains available in domain status.", item.name)),
    });
  }
}

/// Withdraw routes before deleting their application/server dependencies.
/// A failed cleanup retains its tombstone and stops this sync before target deletion.
pub async fn remove(
  resources: &DomainSyncResources,
  user: &User,
  logs: &mut Vec<Log>,
) -> bool {
  for name in &resources.remove {
    let result: anyhow::Result<()> = async {
      execute(
        DomainChangeIntent::Remove {
          domain: name.clone(),
        },
        user,
      )
      .await?;
      let domain = crate::resource::get::<Domain>(name).await?;
      DeleteDomain { id: domain.id }
        .resolve(&WriteArgs { user: user.clone() })
        .await
        .map_err(|e| e.error)?;
      Ok(())
    }
    .await;
    match result {
      Ok(()) => logs.push(Log::simple("Remove domain", format!("Removed owned objects and cleaned Domain {name}; operation audit is retained."))),
      Err(_) => {
        logs.push(Log::error("Remove domain", format!("Cleanup blocked for {name}; tombstone and dependency protections are retained.")));
        return false;
      }
    }
  }
  true
}

async fn execute(
  intent: DomainChangeIntent,
  user: &User,
) -> anyhow::Result<()> {
  let plan = PlanDomainChange { intent }
    .resolve(&ReadArgs { user: user.clone() })
    .await
    .map_err(|e| e.error)?;
  let domain = plan
    .domain_id
    .clone()
    .context("saved domain plan has no ID")?;
  let request = match plan.operation {
    DomainPlanOperation::Remove => {
      ExecuteRequest::RemoveDomain(RemoveDomain {
        domain,
        plan_id: plan.plan_id,
        expected_generation: plan.generation,
        request_id: plan.request_id,
      })
    }
    _ => ExecuteRequest::ReconcileDomain(ReconcileDomain {
      domain,
      plan_id: plan.plan_id,
      expected_generation: plan.generation,
      request_id: plan.request_id,
    }),
  };
  let update = init_execution_update(&request, user).await?;
  let args = ExecuteArgs {
    user: user.clone(),
    update,
    task_id: uuid::Uuid::new_v4(),
  };
  let update = match request {
    ExecuteRequest::RemoveDomain(request) => {
      request.resolve(&args).await
    }
    ExecuteRequest::ReconcileDomain(request) => {
      request.resolve(&args).await
    }
    _ => unreachable!(),
  }
  .map_err(|e| e.error)?;
  let update = tokio::time::timeout(
    std::time::Duration::from_secs(300),
    async {
      loop {
        let current = GetUpdate {
          id: update.id.clone(),
        }
        .resolve(&ReadArgs { user: user.clone() })
        .await
        .map_err(|e| e.error)?;
        if current.end_ts.is_some() {
          return anyhow::Ok(current);
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
      }
    },
  )
  .await
  .context(
    "Domain wait timed out; server operation may continue",
  )??;
  anyhow::ensure!(
    update.success,
    "domain operation did not complete successfully"
  );
  Ok(())
}
