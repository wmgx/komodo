//! Domain commands share the Core API with the web client.
use std::path::PathBuf;

#[derive(Debug, Clone, clap::Parser)]
pub struct Domain {
  #[command(subcommand)]
  pub command: DomainCommand,
  #[arg(long, global = true, value_enum, default_value = "table")]
  pub format: DomainFormat,
  /// Confirm mutations. JSON output does not imply consent.
  #[arg(long, global = true)]
  pub yes: bool,
  #[arg(long, global = true)]
  pub request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
pub enum DomainFormat {
  Table,
  Json,
  Toml,
}
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum InputFormat {
  Json,
  Toml,
}

#[derive(Debug, Clone, clap::Args)]
pub struct FileInput {
  #[arg(long)]
  pub file: PathBuf,
  /// Required when --file - reads standard input.
  #[arg(long, value_enum)]
  pub input_format: Option<InputFormat>,
}

#[derive(Debug, Clone, clap::Args, Default)]
pub struct Filters {
  #[arg(long = "name", short = 'n')]
  pub names: Vec<String>,
  #[arg(long = "tag", short = 't')]
  pub tags: Vec<String>,
}

#[derive(Debug, Clone, clap::Args)]
pub struct WaitOptions {
  #[arg(long)]
  pub wait: bool,
  #[arg(long, default_value = "120s")]
  pub timeout: String,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum ConnectionCommand {
  List(Filters),
  Get {
    name: String,
  },
  Create(FileInput),
  Update {
    name: String,
    #[command(flatten)]
    input: FileInput,
  },
  Delete {
    name: String,
  },
  Validate {
    name: String,
  },
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum DomainCommand {
  Adapters,
  Provider {
    #[command(subcommand)]
    command: ConnectionCommand,
  },
  Ingress {
    #[command(subcommand)]
    command: ConnectionCommand,
  },
  List(Filters),
  Get {
    name: String,
  },
  Create(FileInput),
  Update {
    name: String,
    #[command(flatten)]
    input: FileInput,
    #[arg(long)]
    expected_generation: u64,
  },
  Plan {
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    name: Option<String>,
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long, value_enum)]
    input_format: Option<InputFormat>,
    #[arg(long)]
    out: Option<PathBuf>,
  },
  Apply {
    #[arg(long)]
    plan: PathBuf,
    #[command(flatten)]
    wait: WaitOptions,
  },
  Status {
    name: String,
  },
  Wait {
    name: String,
    #[arg(long)]
    generation: u64,
    #[arg(long, default_value = "120s")]
    timeout: String,
  },
  Export {
    name: String,
  },
  Migrate {
    name: String,
    #[arg(long, required_unless_present = "ingress")]
    dns_provider: Option<String>,
    #[arg(long)]
    ingress: Option<String>,
    #[arg(long)]
    out: Option<PathBuf>,
  },
  FinalizeMigration {
    name: String,
    #[command(flatten)]
    wait: WaitOptions,
  },
  Remove {
    name: String,
    #[arg(long)]
    out: Option<PathBuf>,
  },
  Adopt {
    name: String,
    #[arg(long, required = true)]
    object: Vec<String>,
    #[arg(long)]
    out: Option<PathBuf>,
  },
  Abandon {
    name: String,
    #[arg(long)]
    out: Option<PathBuf>,
  },
}
