use anyhow::{Context, bail};
use colored::Colorize;
use comfy_table::{Attribute, Cell, Table};
use komodo_client::{
  api::{
    execute::ApplyDomain,
    read::{GetDomain, GetDomainPlan},
    write::AddServiceDomain,
  },
  entities::{
    ResourceTarget,
    config::cli::args::domain::{
      DomainAddArgs, DomainAddKind, DomainApplyArgs, DomainCommand,
      DomainPlanArgs, DomainStatusArgs,
    },
    domain::{ChangeAction, DomainPlan, DomainReason, DomainStatus},
    update::{Update, UpdateStatus},
  },
};

use crate::config::cli_config;

pub async fn handle(command: &DomainCommand) -> anyhow::Result<()> {
  match command {
    DomainCommand::Plan(args) => plan(args).await,
    DomainCommand::Apply(args) => apply(args).await,
    DomainCommand::Status(args) => status(args).await,
    DomainCommand::Add(args) => add(args).await,
  }
}

async fn add(args: &DomainAddArgs) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  let target = match args.kind {
    DomainAddKind::Stack => ResourceTarget::Stack(args.name.clone()),
    DomainAddKind::Deployment => {
      ResourceTarget::Deployment(args.name.clone())
    }
  };
  let mut adopt = args.yes;
  loop {
    let res = client
      .write(AddServiceDomain {
        target: target.clone(),
        service: args.service.clone(),
        hostname: args.hostname.clone(),
        port: args.port,
        mode: args.mode,
        adopt_conflicts: adopt,
      })
      .await
      .context("Failed to add domain")?;
    for w in &res.warnings {
      println!("{}: {w}", "Note".yellow());
    }
    if !res.conflicts.is_empty() {
      println!(
        "These remote objects exist but are not managed by Komodo and would be overwritten:"
      );
      for c in &res.conflicts {
        println!("  {} {}", c.object.kind, c.object.key);
      }
      super::wait_for_enter("overwrite them", false)?;
      adopt = true;
      continue;
    }
    if let Some(error) = &res.error {
      bail!("{error}");
    }
    if let Some(domain) = res.domain {
      println!(
        "{} {} ({:?})",
        "Added".green(),
        domain.config.hostname.bold(),
        res.mode.unwrap_or_default()
      );
    }
    return Ok(());
  }
}

async fn plan(args: &DomainPlanArgs) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  let plan = client
    .read(GetDomainPlan {
      domain: args.domain.clone(),
    })
    .await
    .context("Failed to get domain plan")?;
  if args.json {
    println!("{}", serde_json::to_string_pretty(&plan)?);
  } else {
    print_plan(&plan);
  }
  Ok(())
}

async fn apply(args: &DomainApplyArgs) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;

  if !args.yes {
    let plan = client
      .read(GetDomainPlan {
        domain: args.domain.clone(),
      })
      .await
      .context("Failed to get domain plan")?;
    print_plan(&plan);
    if let Some(expected) = &args.plan_id
      && expected != &plan.plan_id
    {
      bail!(
        "Plan changed: expected plan id {expected}, current is {}",
        plan.plan_id
      );
    }
    if !plan.errors.is_empty() {
      bail!("Plan has errors, nothing to apply");
    }
    if !has_work(&plan) {
      println!("\nNo changes to apply.");
      return Ok(());
    }
    if has_conflict(&plan) && !args.adopt_conflicts {
      bail!(
        "Plan has conflicts with objects not managed by Komodo. \
        Review them, then re-run with --adopt-conflicts to take ownership."
      );
    }
    super::wait_for_enter("apply this plan", false)?;
  }

  let update = client
    .execute(ApplyDomain {
      domain: args.domain.clone(),
      plan_id: args.plan_id.clone(),
      adopt_conflicts: args.adopt_conflicts,
    })
    .await
    .context("Failed to apply domain")?;
  let update = if update.status == UpdateStatus::Complete {
    update
  } else {
    client.poll_update_until_complete(&update.id).await?
  };
  print_update(&update);
  if !update.success {
    bail!("Apply failed");
  }
  Ok(())
}

async fn status(args: &DomainStatusArgs) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  let domain = client
    .read(GetDomain {
      domain: args.domain.clone(),
    })
    .await
    .context("Failed to get domain")?;
  let status = domain.info.status;
  if args.json {
    println!(
      "{}",
      serde_json::to_string_pretty(&serde_json::json!({
        "name": domain.name,
        "hostname": domain.config.hostname,
        "status": status,
        "observed_at": domain.info.observed_at,
      }))?
    );
    return Ok(());
  }
  println!("{}: {}", "Domain".dimmed(), domain.name.bold());
  println!("{}: {}", "Hostname".dimmed(), domain.config.hostname);
  match status {
    None => println!("{}: unchecked", "Status".dimmed()),
    Some(DomainStatus::Ready) => {
      println!("{}: {}", "Status".dimmed(), "ready".green())
    }
    Some(DomainStatus::Drifted { reasons }) => {
      println!("{}: {}", "Status".dimmed(), "drifted".yellow());
      reasons.iter().for_each(print_reason);
    }
    Some(DomainStatus::Error { reason }) => {
      println!("{}: {}", "Status".dimmed(), "error".red());
      print_reason(&reason);
    }
  }
  Ok(())
}

fn has_work(plan: &DomainPlan) -> bool {
  plan.changes.iter().any(|c| c.action != ChangeAction::Noop)
}

fn has_conflict(plan: &DomainPlan) -> bool {
  plan
    .changes
    .iter()
    .any(|c| c.action == ChangeAction::Conflict)
}

fn print_reason(reason: &DomainReason) {
  println!("  {}: {}", reason.code.bold(), reason.message);
}

fn fmt_value(value: &Option<serde_json::Value>) -> String {
  value
    .as_ref()
    .map(|v| v.to_string())
    .unwrap_or_else(|| "-".to_string())
}

fn print_plan(plan: &DomainPlan) {
  println!("{}: {}", "Plan".dimmed(), plan.plan_id);
  if !plan.changes.is_empty() {
    let mut table = Table::new();
    table.load_style(comfy_table::presets::UTF8_HORIZONTAL_ONLY);
    table.set_header(
      ["Object", "Action", "Before", "After", "Reason"]
        .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
    );
    for change in &plan.changes {
      table.add_row(vec![
        Cell::new(format!(
          "{} {}",
          change.object.kind, change.object.key
        )),
        Cell::new(format!("{:?}", change.action)),
        Cell::new(fmt_value(&change.before)),
        Cell::new(fmt_value(&change.after)),
        Cell::new(
          change.reason.map(|r| format!("{r:?}")).unwrap_or_default(),
        ),
      ]);
    }
    println!("{table}");
  } else {
    println!("No changes.");
  }
  if !plan.errors.is_empty() {
    println!("{}", "Errors".red());
    plan.errors.iter().for_each(print_reason);
  }
}

fn print_update(update: &Update) {
  println!(
    "Link: '{}'",
    format!("{}/updates/{}", cli_config().host, update.id).bold()
  );
  for log in &update.logs {
    let stage = if log.success {
      log.stage.green()
    } else {
      log.stage.red()
    };
    println!("\n{stage}");
    if !log.stdout.is_empty() {
      println!("{}", log.stdout);
    }
    if !log.stderr.is_empty() {
      eprintln!("{}", log.stderr);
    }
  }
  if update.success {
    println!("\n{}", "APPLY SUCCESSFUL".green());
  } else {
    eprintln!("\n{}", "APPLY FAILED".red());
  }
}
