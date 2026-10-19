use anyhow::{Context, bail};
use colored::Colorize;
use comfy_table::{Attribute, Cell, Table};
use komodo_client::{
  api::{
    execute::ApplyDomain,
    read::{GetDomain, GetDomainPlan, GetServerTunnel, ListServices},
    write::{
      AddServiceDomain, DisableServerTunnel, EnableServerTunnel,
      ExposeService, UnexposeService,
    },
  },
  entities::{
    ResourceTarget,
    config::cli::args::domain::{
      DomainAddArgs, DomainAddKind, DomainApplyArgs, DomainCommand,
      DomainPlanArgs, DomainStatusArgs, ServiceCommand,
      ServiceExposeArgs, ServiceListArgs, ServiceUnexposeArgs,
      TunnelCommand, TunnelDisableArgs, TunnelEnableArgs,
      TunnelStatusArgs,
    },
    domain::{ChangeAction, DomainPlan, DomainReason, DomainStatus},
    domain_account::{ServerTunnel, ServerTunnelState},
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
    DomainCommand::Tunnel { command } => match command {
      TunnelCommand::Enable(args) => tunnel_enable(args).await,
      TunnelCommand::Disable(args) => tunnel_disable(args).await,
      TunnelCommand::Status(args) => tunnel_status(args).await,
    },
  }
}

pub async fn handle_service(
  command: &ServiceCommand,
) -> anyhow::Result<()> {
  match command {
    ServiceCommand::List(args) => service_list(args).await,
    ServiceCommand::Expose(args) => service_expose(args).await,
    ServiceCommand::Unexpose(args) => service_unexpose(args).await,
  }
}

/// `name` is a Deployment, `stack/service` a Stack service.
fn parse_service(input: &str) -> (ResourceTarget, String) {
  match input.split_once('/') {
    Some((stack, service)) => (
      ResourceTarget::Stack(stack.to_string()),
      service.to_string(),
    ),
    None => {
      (ResourceTarget::Deployment(input.to_string()), String::new())
    }
  }
}

async fn service_list(args: &ServiceListArgs) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  let services = client
    .read(ListServices {
      server: args.server.clone(),
    })
    .await
    .context("Failed to list services")?;
  if args.json {
    println!("{}", serde_json::to_string_pretty(&services)?);
    return Ok(());
  }
  let mut table = Table::new();
  table.load_style(comfy_table::presets::UTF8_HORIZONTAL_ONLY);
  table.set_header(
    [
      "Service",
      "Server",
      "State",
      "Ports",
      "LAN URL",
      "Public URL",
    ]
    .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
  );
  for s in &services {
    let name = if s.stack.is_empty() {
      s.name.clone()
    } else {
      format!("{}/{}", s.stack, s.name)
    };
    let join = |items: Vec<String>| {
      if items.is_empty() {
        "-".to_string()
      } else {
        items.join("\n")
      }
    };
    table.add_row(vec![
      Cell::new(name),
      Cell::new(&s.server_name),
      Cell::new(&s.state),
      Cell::new(join(
        s.ports
          .iter()
          .map(|p| format!("{}->{}", p.host, p.container))
          .collect(),
      )),
      Cell::new(join(
        s.ports.iter().filter_map(|p| p.lan_url.clone()).collect(),
      )),
      Cell::new(join(
        s.public.iter().map(|p| p.url.clone()).collect(),
      )),
    ]);
  }
  println!("{table}");
  Ok(())
}

async fn service_expose(
  args: &ServiceExposeArgs,
) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  let (target, service) = parse_service(&args.service);
  println!(
    "Making '{}' public. The first service on a server takes about a minute...",
    args.service
  );
  let mut adopt = false;
  loop {
    let res = client
      .write(ExposeService {
        target: target.clone(),
        service: service.clone(),
        prefix: args.prefix.clone(),
        zone: args.zone.clone(),
        port: args.port,
        adopt_conflicts: adopt,
      })
      .await
      .context("Failed to make the service public")?;
    if !res.conflicts.is_empty() {
      println!(
        "These records already exist but are not managed by Komodo, they would be overwritten:"
      );
      for c in &res.conflicts {
        println!("  {} {}", c.object.kind, c.object.key);
      }
      super::wait_for_enter("overwrite them", args.yes)?;
      adopt = true;
      continue;
    }
    if let Some(error) = res.error {
      bail!("{error}");
    }
    if let Some(url) = res.url {
      println!("{} {}", "Public:".green(), url.bold());
    }
    return Ok(());
  }
}

async fn service_unexpose(
  args: &ServiceUnexposeArgs,
) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  let (target, service) = parse_service(&args.service);
  if !args.yes {
    println!(
      "This removes the public address of '{}'.",
      args.service
    );
    super::wait_for_enter("stop public access", false)?;
  }
  let removed = client
    .write(UnexposeService { target, service })
    .await
    .context("Failed to stop public access")?;
  if removed.is_empty() {
    println!("'{}' was not public.", args.service);
  }
  for hostname in removed {
    println!("{} {hostname}", "Removed".green());
  }
  Ok(())
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

async fn tunnel_enable(
  args: &TunnelEnableArgs,
) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  if !args.yes {
    println!(
      "This creates a Cloudflare Tunnel for '{}' and deploys a cloudflared container on it (Deployment 'cloudflared-{}').",
      args.server, args.server
    );
    super::wait_for_enter("open the tunnel", false)?;
  }
  let tunnel = client
    .write(EnableServerTunnel {
      server: args.server.clone(),
      account: args.account.clone(),
      image: args.image.clone(),
    })
    .await
    .context("Failed to open the tunnel")?;
  print_tunnel(&tunnel);
  println!(
    "\nThe connector may need a few seconds to connect. Check with `km domain tunnel status {}`.",
    args.server
  );
  Ok(())
}

async fn tunnel_disable(
  args: &TunnelDisableArgs,
) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  if !args.yes {
    println!(
      "This removes the cloudflared Deployment of '{}'{}.",
      args.server,
      if args.keep_remote {
        ""
      } else {
        " and deletes its tunnel at Cloudflare"
      }
    );
    super::wait_for_enter("close the tunnel", false)?;
  }
  let tunnel = client
    .write(DisableServerTunnel {
      server: args.server.clone(),
      delete_remote: Some(!args.keep_remote),
    })
    .await
    .context("Failed to close the tunnel")?;
  print_tunnel(&tunnel);
  Ok(())
}

async fn tunnel_status(
  args: &TunnelStatusArgs,
) -> anyhow::Result<()> {
  let client = super::komodo_client().await?;
  let tunnel = client
    .read(GetServerTunnel {
      server: args.server.clone(),
    })
    .await
    .context("Failed to get the tunnel")?;
  if args.json {
    println!("{}", serde_json::to_string_pretty(&tunnel)?);
  } else {
    print_tunnel(&tunnel);
  }
  Ok(())
}

fn print_tunnel(tunnel: &ServerTunnel) {
  let state = match tunnel.state {
    ServerTunnelState::NotEnabled => "not enabled".dimmed(),
    ServerTunnelState::Starting => "starting".yellow(),
    ServerTunnelState::Healthy => "healthy".green(),
    ServerTunnelState::Degraded => "degraded".yellow(),
    ServerTunnelState::Down => "down".red(),
    ServerTunnelState::Inactive => "inactive".yellow(),
  };
  println!("{}: {state}", "Tunnel".dimmed());
  if let Some(name) = &tunnel.tunnel_name {
    println!("{}: {name}", "Name".dimmed());
  }
  if let Some(account) = &tunnel.account {
    println!("{}: {account}", "Account".dimmed());
  }
  if let Some(deployment) = &tunnel.deployment {
    println!(
      "{}: {deployment} ({:?})",
      "Connector".dimmed(),
      tunnel.deployment_state.unwrap_or_default()
    );
  }
  if tunnel.state == ServerTunnelState::NotEnabled {
    println!("{}: {}", "Image".dimmed(), tunnel.image);
  }
  if !tunnel.domains.is_empty() {
    println!("{}: {}", "Domains".dimmed(), tunnel.domains.join(", "));
  }
  if let Some(warning) = &tunnel.warning {
    println!("{}: {warning}", "Note".yellow());
  }
}
