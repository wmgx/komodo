#[derive(Debug, Clone, clap::Subcommand)]
pub enum DomainCommand {
  /// Show the structured plan for a Domain, without changing anything.
  Plan(DomainPlanArgs),
  /// Apply a Domain. Shows the plan and asks for confirmation first.
  Apply(DomainApplyArgs),
  /// Show the last observed status of a Domain.
  Status(DomainStatusArgs),
  /// Give a Deployment / Stack service a hostname in one step.
  Add(DomainAddArgs),
  /// Open / close / inspect the Cloudflare Tunnel of a Server.
  Tunnel {
    #[command(subcommand)]
    command: TunnelCommand,
  },
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum TunnelCommand {
  /// Create a tunnel and run its connector (cloudflared) on the Server.
  Enable(TunnelEnableArgs),
  /// Remove the connector and the tunnel Komodo opened for the Server.
  Disable(TunnelDisableArgs),
  /// Show whether the Server has a tunnel and if it is connected.
  Status(TunnelStatusArgs),
}

#[derive(Debug, Clone, clap::Parser)]
pub struct TunnelEnableArgs {
  /// Server name or id.
  pub server: String,
  /// Domain account to use. Optional when only one can open tunnels.
  #[arg(long)]
  pub account: Option<String>,
  /// Connector image, eg. docker.1ms.run/cloudflare/cloudflared:latest.
  /// Remembered on the account for next time.
  #[arg(long)]
  pub image: Option<String>,
  /// Don't ask for confirmation.
  #[arg(long, short = 'y', default_value_t = false)]
  pub yes: bool,
}

#[derive(Debug, Clone, clap::Parser)]
pub struct TunnelDisableArgs {
  /// Server name or id.
  pub server: String,
  /// Keep the tunnel at Cloudflare, only remove the connector and binding.
  #[arg(long, default_value_t = false)]
  pub keep_remote: bool,
  /// Don't ask for confirmation.
  #[arg(long, short = 'y', default_value_t = false)]
  pub yes: bool,
}

#[derive(Debug, Clone, clap::Parser)]
pub struct TunnelStatusArgs {
  /// Server name or id.
  pub server: String,
  /// Print the status as JSON.
  #[arg(long, default_value_t = false)]
  pub json: bool,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum DomainAddKind {
  Stack,
  Deployment,
}

#[derive(Debug, Clone, clap::Parser)]
pub struct DomainAddArgs {
  /// `stack` or `deployment`.
  pub kind: DomainAddKind,
  /// Stack / Deployment name or id.
  pub name: String,
  /// Full hostname, eg. app.example.com
  pub hostname: String,
  /// Compose service name (Stacks only).
  #[arg(long, default_value = "")]
  pub service: String,
  /// Published port. Detected from Deployment config when omitted.
  #[arg(long, default_value_t = 0)]
  pub port: u16,
  /// auto (default), tunnel or dns.
  #[arg(long, value_enum, default_value_t = crate::entities::domain::DomainMode::Auto)]
  pub mode: crate::entities::domain::DomainMode,
  /// Overwrite existing remote records which Komodo does not own,
  /// without asking.
  #[arg(long, short = 'y', default_value_t = false)]
  pub yes: bool,
}

#[derive(Debug, Clone, clap::Parser)]
pub struct DomainPlanArgs {
  /// Domain name or id.
  pub domain: String,
  /// Print the raw plan JSON.
  #[arg(long, default_value_t = false)]
  pub json: bool,
}

#[derive(Debug, Clone, clap::Parser)]
pub struct DomainApplyArgs {
  /// Domain name or id.
  pub domain: String,
  /// Only apply if the current plan has this id (from `km domain plan`).
  #[arg(long)]
  pub plan_id: Option<String>,
  /// Take ownership of existing remote objects not created by Komodo.
  #[arg(long, default_value_t = false)]
  pub adopt_conflicts: bool,
  /// Skip the plan preview and confirmation prompt.
  #[arg(long, short = 'y', default_value_t = false)]
  pub yes: bool,
}

#[derive(Debug, Clone, clap::Parser)]
pub struct DomainStatusArgs {
  /// Domain name or id.
  pub domain: String,
  /// Print the status as JSON.
  #[arg(long, default_value_t = false)]
  pub json: bool,
}

#[cfg(test)]
mod tests {
  use clap::Parser;

  use super::super::{CliArgs, Command};
  use super::DomainCommand;

  #[test]
  fn parses_domain_apply() {
    let args = CliArgs::try_parse_from([
      "km",
      "domain",
      "apply",
      "app",
      "--plan-id",
      "abc",
      "--adopt-conflicts",
      "-y",
    ])
    .unwrap();
    let Command::Domain {
      command: DomainCommand::Apply(apply),
    } = args.command
    else {
      panic!("expected domain apply");
    };
    assert_eq!(apply.domain, "app");
    assert_eq!(apply.plan_id.as_deref(), Some("abc"));
    assert!(apply.adopt_conflicts && apply.yes);
  }
}
