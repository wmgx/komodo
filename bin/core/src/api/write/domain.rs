use anyhow::{Context, anyhow};
use database::mungos::mongodb::bson::{doc, oid::ObjectId};
use komodo_client::{
  api::write::*,
  entities::{
    Operation, ResourceTarget,
    domain::{Domain, ExposeServiceResult, ServiceDomainResult},
    domain_account::{DomainAccount, DomainAccountDoc, ServerTunnel},
    permission::PermissionLevel,
    update::Update,
  },
};
use mogh_error::AddStatusCodeError;
use mogh_resolver::Resolve;
use reqwest::StatusCode;

use crate::{
  domain::{
    accounts::{self, ResolvedAccount},
    error::DomainError,
    expose::{ExposeRequest, expose, unexpose},
    provider::factory,
    service::{AddRequest, add_service_domain},
    tunnel::{self, EnableRequest},
  },
  helpers::update::{add_update, make_update},
  permission::get_check_permissions,
  resource,
  state::db_client,
};

use super::WriteArgs;

impl Resolve<WriteArgs> for CreateDomain {
  #[instrument(
    "CreateDomain",
    skip_all,
    fields(operator = user.id, domain = self.name)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Domain> {
    resource::create::<Domain>(&self.name, self.config, None, user)
      .await
  }
}

impl Resolve<WriteArgs> for AddServiceDomain {
  #[instrument(
    "AddServiceDomain",
    skip_all,
    fields(operator = user.id, hostname = self.hostname)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<ServiceDomainResult> {
    require_admin(user, "add service domains")?;
    add_service_domain(
      AddRequest {
        target: self.target,
        service: self.service,
        hostname: self.hostname,
        port: self.port,
        mode: self.mode,
        adopt_conflicts: self.adopt_conflicts,
      },
      user,
    )
    .await
    .map_err(bad_request)
  }
}

impl Resolve<WriteArgs> for ExposeService {
  #[instrument(
    "ExposeService",
    skip_all,
    fields(operator = user.id, service = self.service)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<ExposeServiceResult> {
    require_admin(user, "make services public")?;
    expose(
      ExposeRequest {
        target: self.target,
        service: self.service,
        prefix: self.prefix,
        zone: self.zone,
        port: self.port,
        adopt_conflicts: self.adopt_conflicts,
      },
      user,
    )
    .await
    .map_err(|e| {
      e.log("ExposeService");
      bad_request(e)
    })
  }
}

impl Resolve<WriteArgs> for UnexposeService {
  #[instrument(
    "UnexposeService",
    skip_all,
    fields(operator = user.id, service = self.service)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Vec<String>> {
    require_admin(user, "stop public access")?;
    unexpose(&self.target, &self.service, user)
      .await
      .map_err(|e| {
        e.log("UnexposeService");
        bad_request(e)
      })
  }
}

impl Resolve<WriteArgs> for EnableServerTunnel {
  #[instrument(
    "EnableServerTunnel",
    skip_all,
    fields(operator = user.id, server = self.server)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<ServerTunnel> {
    require_admin(user, "open server connections")?;
    tunnel::enable(
      EnableRequest {
        server: self.server,
        account: self.account.filter(|a| !a.is_empty()),
        image: self.image,
      },
      user,
    )
    .await
    .map_err(|e| {
      e.log("EnableServerTunnel");
      e.into_api()
    })
  }
}

impl Resolve<WriteArgs> for DisableServerTunnel {
  #[instrument(
    "DisableServerTunnel",
    skip_all,
    fields(operator = user.id, server = self.server)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<ServerTunnel> {
    require_admin(user, "close server connections")?;
    tunnel::disable(
      &self.server,
      self.delete_remote.unwrap_or(true),
      user,
    )
    .await
    .map_err(|e| {
      e.log("DisableServerTunnel");
      e.into_api()
    })
  }
}

impl Resolve<WriteArgs> for CopyDomain {
  #[instrument(
    "CopyDomain",
    skip_all,
    fields(operator = user.id, domain = self.name, copy_domain = self.id)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Domain> {
    let Domain { config, .. } = get_check_permissions::<Domain>(
      &self.id,
      user,
      PermissionLevel::Write.into(),
    )
    .await?;
    resource::create::<Domain>(&self.name, config.into(), None, user)
      .await
  }
}

impl Resolve<WriteArgs> for DeleteDomain {
  #[instrument(
    "DeleteDomain",
    skip_all,
    fields(operator = user.id, domain = self.id)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Domain> {
    Ok(resource::delete::<Domain>(&self.id, user).await?)
  }
}

impl Resolve<WriteArgs> for UpdateDomain {
  #[instrument(
    "UpdateDomain",
    skip_all,
    fields(operator = user.id, domain = self.id)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Domain> {
    Ok(resource::update::<Domain>(&self.id, self.config, user).await?)
  }
}

impl Resolve<WriteArgs> for RenameDomain {
  #[instrument(
    "RenameDomain",
    skip_all,
    fields(operator = user.id, domain = self.id, new_name = self.name)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<Update> {
    Ok(resource::rename::<Domain>(&self.id, &self.name, user).await?)
  }
}

// ACCOUNTS (admin only, tokens are write-only)

fn require_admin(
  user: &komodo_client::entities::user::User,
  what: &str,
) -> mogh_error::Result<()> {
  if user.admin {
    Ok(())
  } else {
    Err(
      anyhow!("Only admins can {what}")
        .status_code(StatusCode::FORBIDDEN),
    )
  }
}

fn bad_request(e: DomainError) -> mogh_error::Error {
  e.status_code(StatusCode::BAD_REQUEST)
}

async fn log_update(
  user: &komodo_client::entities::user::User,
  operation: Operation,
  stage: &str,
  message: String,
) {
  let mut update =
    make_update(ResourceTarget::system(), operation, user);
  update.push_simple_log(stage, message);
  update.finalize();
  if let Err(e) = add_update(update).await {
    error!("Failed to add update for {stage} | {e:#}");
  }
}

impl Resolve<WriteArgs> for CreateDomainAccount {
  #[instrument(
    "CreateDomainAccount",
    skip_all,
    fields(operator = user.id, name = self.name, kind = self.kind)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<DomainAccount> {
    require_admin(user, "create domain accounts")?;
    if self.name.trim().is_empty() {
      return Err(
        anyhow!("Name cannot be empty")
          .status_code(StatusCode::BAD_REQUEST),
      );
    }
    if self.token.is_empty() {
      return Err(
        anyhow!("Token cannot be empty")
          .status_code(StatusCode::BAD_REQUEST),
      );
    }
    let factory = factory(&self.kind).ok_or_else(|| {
      anyhow!("Unknown provider kind '{}'", self.kind)
        .status_code(StatusCode::BAD_REQUEST)
    })?;
    (factory.validate_params)(&self.params).map_err(bad_request)?;

    let mut doc = DomainAccountDoc {
      id: String::new(),
      name: self.name,
      kind: self.kind,
      token: self.token,
      params: self.params,
      description: self.description,
    };
    doc.id = db_client()
      .domain_accounts
      .insert_one(&doc)
      .await
      .context(
        "Failed to create domain account on db. The name may already be taken.",
      )?
      .inserted_id
      .as_object_id()
      .context("Inserted id is not ObjectId")?
      .to_string();

    log_update(
      user,
      Operation::CreateDomainAccount,
      "Create domain account",
      format!("Created {} domain account '{}'", doc.kind, doc.name),
    )
    .await;
    Ok(ResolvedAccount::from(doc).view())
  }
}

impl Resolve<WriteArgs> for UpdateDomainAccount {
  #[instrument(
    "UpdateDomainAccount",
    skip_all,
    fields(operator = user.id, id = self.id)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<DomainAccount> {
    require_admin(user, "update domain accounts")?;
    let account = accounts::find_by_id_or_name(&self.id)
      .await
      .map_err(|e| e.into_api())?
      .ok_or_else(|| {
        anyhow!("No domain account matching '{}'", self.id)
          .status_code(StatusCode::NOT_FOUND)
      })?;
    if account.id.is_empty() {
      return Err(
        anyhow!("Accounts declared in the Core config are read only")
          .status_code(StatusCode::BAD_REQUEST),
      );
    }

    let mut set = doc! {};
    if let Some(token) = self.token.filter(|t| !t.is_empty()) {
      set.insert("token", token);
    }
    if let Some(params) = self.params {
      (account.factory().map_err(bad_request)?.validate_params)(
        &params,
      )
      .map_err(bad_request)?;
      set.insert(
        "params",
        database::bson::to_bson(&params)
          .context("Failed to serialize params")?,
      );
    }
    if let Some(description) = self.description {
      set.insert("description", description);
    }
    if !set.is_empty() {
      let oid = ObjectId::parse_str(&account.id)
        .context("Invalid account id")?;
      db_client()
        .domain_accounts
        .update_one(doc! { "_id": oid }, doc! { "$set": set })
        .await
        .context("Failed to update domain account on db")?;
    }

    log_update(
      user,
      Operation::UpdateDomainAccount,
      "Update domain account",
      format!("Updated domain account '{}'", account.name),
    )
    .await;
    let updated = accounts::find_by_id_or_name(&account.id)
      .await
      .map_err(|e| e.into_api())?
      .context("Account disappeared after update")?;
    Ok(updated.view())
  }
}

impl Resolve<WriteArgs> for DeleteDomainAccount {
  #[instrument(
    "DeleteDomainAccount",
    skip_all,
    fields(operator = user.id, id = self.id)
  )]
  async fn resolve(
    self,
    WriteArgs { user }: &WriteArgs,
  ) -> mogh_error::Result<DomainAccount> {
    require_admin(user, "delete domain accounts")?;
    let account = accounts::find_by_id_or_name(&self.id)
      .await
      .map_err(|e| e.into_api())?
      .ok_or_else(|| {
        anyhow!("No domain account matching '{}'", self.id)
          .status_code(StatusCode::NOT_FOUND)
      })?;
    if account.id.is_empty() {
      return Err(
        anyhow!("Accounts declared in the Core config cannot be deleted here")
          .status_code(StatusCode::BAD_REQUEST),
      );
    }
    let in_use = db_client()
      .domains
      .find_one(doc! { "$or": [
        { "config.dns.account": &account.name },
        { "config.ingress.account": &account.name },
        { "info.owned.account": &account.name },
      ] })
      .await
      .context("Failed to query domains using the account")?;
    if let Some(domain) = in_use {
      return Err(
        anyhow!(
          "Account is still used by Domain '{}'. Update or delete it first.",
          domain.name
        )
        .status_code(StatusCode::CONFLICT),
      );
    }
    let oid = ObjectId::parse_str(&account.id)
      .context("Invalid account id")?;
    db_client()
      .domain_accounts
      .delete_one(doc! { "_id": oid })
      .await
      .context("Failed to delete domain account on db")?;

    log_update(
      user,
      Operation::DeleteDomainAccount,
      "Delete domain account",
      format!("Deleted domain account '{}'", account.name),
    )
    .await;
    Ok(account.view())
  }
}
