#[derive(Debug, Clone, clap::Subcommand)]
pub enum DomainCommand {
  /// Show the structured plan for a Domain, without changing anything.
  Plan(DomainPlanArgs),
  /// Apply a Domain. Shows the plan and asks for confirmation first.
  Apply(DomainApplyArgs),
  /// Show the last observed status of a Domain.
  Status(DomainStatusArgs),
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
