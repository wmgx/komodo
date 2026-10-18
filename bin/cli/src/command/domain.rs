//! Domain CLI orchestration. All authorization, planning and external writes belong to Core.
use komodo_client::{
  KomodoClient,
  api::{execute::*, read::*, write::*},
  entities::{
    config::cli::args::domain::{
      ConnectionCommand, Domain as DomainArgs, DomainCommand,
      DomainFormat, FileInput, Filters, InputFormat, WaitOptions,
    },
    domain::*,
  },
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
  io::{IsTerminal, Read, Write},
  path::Path,
  time::Duration,
};

#[derive(Debug)]
pub struct Failure(pub DomainError);
impl std::fmt::Display for Failure {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.0.message)
  }
}
impl std::error::Error for Failure {}
fn fail(code: &str, message: &str) -> anyhow::Error {
  Failure(DomainError {
    code: code.into(),
    message: message.into(),
    retryable: false,
    field: None,
    update_id: None,
  })
  .into()
}
pub fn exit_code(code: &str) -> i32 {
  match code {
    "InvalidInput"
    | "InvalidSchema"
    | "UnsupportedCapability"
    | "UnsupportedAdapter"
    | "InvalidConfig"
    | "InvalidHostname"
    | "InvalidPlan"
    | "InvalidRequestId"
    | "PlanRequired"
    | "MigrationRequired" => 2,
    "StalePlan"
    | "GenerationConflict"
    | "Drifted"
    | "Conflict"
    | "IdempotencyConflict" => 3,
    "Unauthorized" | "Forbidden" | "PermissionDenied" => 4,
    "Timeout" => 5,
    _ => 1,
  }
}
fn error(e: &anyhow::Error) -> DomainError {
  if let Some(f) = e.downcast_ref::<Failure>() {
    return f.0.clone();
  }
  for cause in e.chain() {
    if let Ok(e) =
      serde_json::from_str::<DomainError>(&cause.to_string())
    {
      return e;
    }
    let text = cause.to_string();
    let code = text.split(':').next().unwrap_or_default();
    if matches!(
      code,
      "InvalidInput"
        | "InvalidSchema"
        | "InvalidConfig"
        | "InvalidHostname"
        | "InvalidPlan"
        | "InvalidRequestId"
        | "UnsupportedCapability"
        | "UnsupportedAdapter"
        | "PlanRequired"
        | "MigrationRequired"
        | "StalePlan"
        | "GenerationConflict"
        | "Drifted"
        | "Conflict"
        | "IdempotencyConflict"
        | "Unauthorized"
        | "Forbidden"
        | "PermissionDenied"
        | "NotFound"
        | "FeatureDisabled"
        | "OwnershipUnconfirmed"
        | "OperationRecoveryRequired"
        | "LeaseLost"
        | "DrainPending"
        | "IngressUnavailable"
        | "VerificationPending"
        | "RateLimited"
        | "ProviderTimeout"
        | "ProviderUnavailable"
    ) {
      return DomainError {
        code: code.into(),
        message: format!(
          "Core rejected the domain request ({code}); inspect the resource status and replan if needed"
        ),
        retryable: matches!(
          code,
          "DrainPending"
            | "IngressUnavailable"
            | "VerificationPending"
            | "RateLimited"
            | "ProviderTimeout"
            | "ProviderUnavailable"
        ),
        field: None,
        update_id: None,
      };
    }
  }
  // Do not print opaque provider response bodies or request headers.
  let code = if e.chain().any(|c| c.to_string().starts_with("401")) {
    "Unauthorized"
  } else if e.chain().any(|c| c.to_string().starts_with("403")) {
    "Forbidden"
  } else {
    "ExecutionFailed"
  };
  DomainError {code:code.into(),message:"Domain request failed; inspect the sanitized operation status in Core".into(),retryable:false,field:None,update_id:None}
}
fn envelope(data: Value, error: Option<DomainError>) -> Value {
  json!({"api_version":"domain.v1","ok":error.is_none(),"data":data,"error":error})
}
pub async fn handle(args: &DomainArgs) -> anyhow::Result<()> {
  match run(args).await {
    Ok(value) => {
      if args.format == DomainFormat::Toml {
        println!("{}", toml::to_string_pretty(&value)?);
      } else if args.format == DomainFormat::Json {
        println!("{}", envelope(value, None));
      } else {
        print_human(&value)?;
      }
      Ok(())
    }
    Err(e) => {
      let e = error(&e);
      let code = exit_code(&e.code);
      if args.format == DomainFormat::Json {
        println!("{}", envelope(Value::Null, Some(e)));
      } else {
        eprintln!("{}: {}", e.code, e.message);
      }
      std::process::exit(code)
    }
  }
}

fn print_human(value: &Value) -> anyhow::Result<()> {
  if let Some(items) = value.as_array() {
    let mut table = comfy_table::Table::new();
    table.load_style(comfy_table::presets::UTF8_HORIZONTAL_ONLY);
    table.set_header([
      "Name / adapter",
      "Hostname",
      "State / schema",
    ]);
    for item in items {
      table.add_row([
        item["name"]
          .as_str()
          .or(item["adapter"].as_str())
          .unwrap_or(""),
        item["config"]["hostname"].as_str().unwrap_or(""),
        item["info"]["state"].as_str().unwrap_or(""),
      ]);
    }
    println!("{table}");
  } else if let Some(state) = value["state"].as_str() {
    println!(
      "{}: {} (generation {})",
      value["domain_id"].as_str().unwrap_or("domain"),
      state,
      value["generation"]
    );
    if let Some(update) = value["update_id"].as_str() {
      println!("Update: {update}");
    }
    if let Some(conditions) = value["conditions"].as_array() {
      for condition in conditions {
        println!(
          "{}: {} — {}",
          condition["condition"].as_str().unwrap_or(""),
          condition["status"].as_str().unwrap_or("Unknown"),
          condition["message"].as_str().unwrap_or("")
        );
      }
    }
  } else if value.get("plan_id").is_some() {
    println!(
      "Plan {}: {} {} (generation {})",
      value["plan_id"].as_str().unwrap_or(""),
      value["operation"].as_str().unwrap_or(""),
      value["name"].as_str().unwrap_or(""),
      value["generation"]
    );
    for field in ["steps", "warnings"] {
      if let Some(items) = value[field].as_array() {
        for item in items {
          println!("- {}", item.as_str().unwrap_or(""));
        }
      }
    }
    if value["requires_downtime"] == true {
      println!("This migration requires downtime.");
    }
  } else {
    println!("{}", serde_json::to_string_pretty(value)?);
  }
  Ok(())
}

fn confirm(yes: bool, summary: &str) -> anyhow::Result<()> {
  if yes {
    return Ok(());
  }
  if !std::io::stdin().is_terminal() {
    return Err(fail(
      "ConfirmationRequired",
      "Non-interactive writes require --yes",
    ));
  }
  eprint!("{summary}\nContinue? [y/N] ");
  std::io::stderr().flush()?;
  let mut line = String::new();
  std::io::stdin().read_line(&mut line)?;
  if !matches!(line.trim(), "y" | "Y" | "yes") {
    return Err(fail("Cancelled", "No changes requested"));
  }
  Ok(())
}
fn duration(s: &str) -> anyhow::Result<Duration> {
  let (n, mult) = if let Some(n) = s.strip_suffix("ms") {
    (n, 1)
  } else if let Some(n) = s.strip_suffix('s') {
    (n, 1000)
  } else if let Some(n) = s.strip_suffix('m') {
    (n, 60000)
  } else {
    return Err(fail("InvalidInput", "Timeout must use ms, s or m"));
  };
  let millis = n
    .parse::<u64>()
    .ok()
    .and_then(|n| n.checked_mul(mult))
    .filter(|n| *n > 0)
    .ok_or_else(|| fail("InvalidInput", "Invalid timeout"))?;
  Ok(Duration::from_millis(millis))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition<C> {
  name: String,
  config: C,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  expected_generation: Option<i64>,
}
fn parse_input<T: DeserializeOwned>(
  text: &str,
  format: InputFormat,
) -> anyhow::Result<T> {
  match format {
    InputFormat::Json => serde_json::from_str(text).map_err(|_| {
      fail(
        "InvalidSchema",
        "Input is not a single valid resource definition",
      )
    }),
    InputFormat::Toml => toml::from_str(text).map_err(|_| {
      fail(
        "InvalidSchema",
        "Input is not a single valid resource definition",
      )
    }),
  }
}
fn input<T: DeserializeOwned>(
  input: &FileInput,
) -> anyhow::Result<T> {
  let stdin = input.file == Path::new("-");
  let format=input.input_format.or_else(||match input.file.extension().and_then(|s|s.to_str()) {Some("json")=>Some(InputFormat::Json),Some("toml")=>Some(InputFormat::Toml),_=>None}).ok_or_else(||fail("InvalidInput","Use a .json/.toml file; stdin requires --input-format json|toml"))?;
  let text = if stdin {
    let mut text = String::new();
    std::io::stdin()
      .take(4 * 1024 * 1024 + 1)
      .read_to_string(&mut text)?;
    text
  } else {
    std::fs::read_to_string(&input.file)
      .map_err(|_| fail("InvalidInput", "Cannot read input file"))?
  };
  if text.len() > 4 * 1024 * 1024 {
    return Err(fail("InvalidInput", "Input exceeds 4 MiB"));
  }
  parse_input(&text, format)
}
async fn client() -> anyhow::Result<&'static KomodoClient> {
  super::komodo_client().await
}
fn value(v: impl Serialize) -> anyhow::Result<Value> {
  Ok(serde_json::to_value(v)?)
}
async fn plan(
  intent: DomainChangeIntent,
  out: Option<&Path>,
  request_id: Option<&str>,
) -> anyhow::Result<Value> {
  let mut plan =
    client().await?.read(PlanDomainChange { intent }).await?;
  plan.request_id = request_id
    .map(str::to_owned)
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
  if let Some(out) = out {
    save_plan(out, &plan)?;
  }
  value(plan)
}
fn save_plan(path: &Path, plan: &DomainPlan) -> anyhow::Result<()> {
  // Atomic replacement means retries always see one complete reference.
  let parent = path
    .parent()
    .filter(|p| !p.as_os_str().is_empty())
    .unwrap_or(Path::new("."));
  let tmp =
    parent.join(format!(".domain-plan-{}.tmp", uuid::Uuid::new_v4()));
  let result = (|| {
    let mut file = std::fs::OpenOptions::new()
      .write(true)
      .create_new(true)
      .open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(plan)?)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok::<_, anyhow::Error>(())
  })();
  if result.is_err() {
    let _ = std::fs::remove_file(tmp);
  }
  result
}
fn filtered(mut data: Value, filters: &Filters) -> Value {
  if let Some(items) = data.as_array_mut() {
    let patterns = super::parse_wildcards(&filters.names);
    items.retain(|v| {
      super::matches_wildcards(
        &patterns,
        &[v["name"].as_str().unwrap_or_default()],
      )
    });
  }
  data
}
async fn run(args: &DomainArgs) -> anyhow::Result<Value> {
  if args.format == DomainFormat::Toml
    && !matches!(args.command, DomainCommand::Export { .. })
  {
    return Err(fail(
      "InvalidInput",
      "TOML output is only supported by export",
    ));
  }
  use DomainCommand::*;
  match &args.command {
    Adapters => {
      value(client().await?.read(ListDomainAdapters {}).await?)
    }
    Provider { command } => connection(command, true, args.yes).await,
    Ingress { command } => connection(command, false, args.yes).await,
    List(filters) => value(filtered(
      value(
        client()
          .await?
          .read(ListDomains {
            query: DomainQuery {
              tags: filters.tags.clone(),
              ..Default::default()
            },
            page: 0,
            limit: Some(0),
          })
          .await?,
      )?,
      filters,
    )),
    Get { name } => value(
      client()
        .await?
        .read(GetDomain {
          domain: name.clone(),
        })
        .await?,
    ),
    Status { name } => value(
      client()
        .await?
        .read(GetDomainStatus {
          domain: name.clone(),
        })
        .await?,
    ),
    Create(file) => {
      let def: Definition<DomainConfig> = input(file)?;
      confirm(
        args.yes,
        &format!("Create Pending domain {}", def.name),
      )?;
      value(
        client()
          .await?
          .write(CreateDomain {
            name: def.name,
            config: def.config,
          })
          .await?,
      )
    }
    Update {
      name,
      input: file,
      expected_generation,
    } => {
      let def: Definition<DomainConfig> = input(file)?;
      confirm(
        args.yes,
        &format!("Save desired configuration for {name}"),
      )?;
      value(
        client()
          .await?
          .write(UpdateDomain {
            id: name.clone(),
            config: def.config,
            expected_generation: i64::try_from(*expected_generation)
              .map_err(|_| {
                fail("InvalidInput", "Generation out of range")
              })?,
          })
          .await?,
      )
    }
    Plan {
      name,
      file,
      input_format,
      out,
    } => {
      let intent = if let Some(file) = file {
        let def: Definition<DomainConfig> = input(&FileInput {
          file: file.clone(),
          input_format: *input_format,
        })?;
        match def.expected_generation {
          Some(expected_generation) => DomainChangeIntent::Update {
            domain: def.name,
            config: def.config,
            expected_generation,
          },
          None => DomainChangeIntent::Create {
            name: def.name,
            config: def.config,
          },
        }
      } else {
        DomainChangeIntent::Reconcile {
          domain: name.clone().ok_or_else(|| {
            fail("InvalidInput", "Specify domain or --file")
          })?,
        }
      };
      plan(intent, out.as_deref(), args.request_id.as_deref()).await
    }
    Migrate {
      name,
      dns_provider,
      ingress,
      out,
    } => {
      plan(
        DomainChangeIntent::Migrate {
          domain: name.clone(),
          dns_provider: dns_provider.clone(),
          ingress: ingress.clone(),
        },
        out.as_deref(),
        args.request_id.as_deref(),
      )
      .await
    }
    Remove { name, out } => {
      plan(
        DomainChangeIntent::Remove {
          domain: name.clone(),
        },
        out.as_deref(),
        args.request_id.as_deref(),
      )
      .await
    }
    Adopt { name, object, out } => {
      plan(
        DomainChangeIntent::Adopt {
          domain: name.clone(),
          objects: object.clone(),
        },
        out.as_deref(),
        args.request_id.as_deref(),
      )
      .await
    }
    Abandon { name, out } => {
      plan(
        DomainChangeIntent::Abandon {
          domain: name.clone(),
        },
        out.as_deref(),
        args.request_id.as_deref(),
      )
      .await
    }
    Apply { plan: path, wait } => {
      let mut reference: DomainPlan = parse_input(
        &std::fs::read_to_string(path).map_err(|_| {
          fail("InvalidInput", "Cannot read plan file")
        })?,
        InputFormat::Json,
      )?;
      if let Some(id) = &args.request_id {
        reference.request_id = id.clone();
      } else if reference.request_id.is_empty() {
        reference.request_id = uuid::Uuid::new_v4().to_string();
      }
      save_plan(path, &reference)?;
      apply(reference, args.yes, wait).await
    }
    Wait {
      name,
      generation,
      timeout,
    } => {
      wait_for(
        name,
        i64::try_from(*generation).map_err(|_| {
          fail("InvalidInput", "Generation out of range")
        })?,
        duration(timeout)?,
      )
      .await
    }
    FinalizeMigration { name, wait } => {
      let p = plan(
        DomainChangeIntent::FinalizeMigration {
          domain: name.clone(),
        },
        None,
        args.request_id.as_deref(),
      )
      .await?;
      apply(serde_json::from_value(p)?, args.yes, wait).await
    }
    Export { name } => {
      let mut domain = client()
        .await?
        .read(GetDomain {
          domain: name.clone(),
        })
        .await?;
      domain.config.ingress = client()
        .await?
        .read(GetIngress {
          ingress: domain.config.ingress.clone(),
        })
        .await?
        .name;
      if let DomainDns::Managed { provider, .. } =
        &mut domain.config.dns
      {
        *provider = client()
          .await?
          .read(GetDnsProvider {
            dns_provider: provider.clone(),
          })
          .await?
          .name;
      }
      match &mut domain.config.target {
        DomainTarget::Deployment { id, .. } => {
          *id = client()
            .await?
            .read(GetDeployment {
              deployment: id.clone(),
            })
            .await?
            .name
        }
        DomainTarget::StackService { stack_id, .. } => {
          *stack_id = client()
            .await?
            .read(GetStack {
              stack: stack_id.clone(),
            })
            .await?
            .name
        }
        _ => {}
      }
      value(Definition {
        name: domain.name,
        config: domain.config,
        expected_generation: None,
      })
    }
  }
}
async fn connection(
  command: &ConnectionCommand,
  provider: bool,
  yes: bool,
) -> anyhow::Result<Value> {
  use ConnectionCommand::*;
  let c = client().await?;
  match command {
    List(filters) => {
      let data = if provider {
        value(
          c.read(ListDnsProviders {
            query: DomainQuery {
              tags: filters.tags.clone(),
              ..Default::default()
            },
            page: 0,
            limit: Some(0),
          })
          .await?,
        )?
      } else {
        value(
          c.read(ListIngresses {
            query: DomainQuery {
              tags: filters.tags.clone(),
              ..Default::default()
            },
            page: 0,
            limit: Some(0),
          })
          .await?,
        )?
      };
      Ok(filtered(data, filters))
    }
    Get { name } => {
      if provider {
        value(
          c.read(GetDnsProvider {
            dns_provider: name.clone(),
          })
          .await?,
        )
      } else {
        value(
          c.read(GetIngress {
            ingress: name.clone(),
          })
          .await?,
        )
      }
    }
    Validate { name } => {
      let result = c
        .read(ValidateDomainConnection {
          connection: if provider {
            DomainConnection::DnsProvider(name.clone())
          } else {
            DomainConnection::Ingress(name.clone())
          },
        })
        .await?;
      if !result.valid {
        return Err(
          Failure(result.errors.first().cloned().unwrap_or(
            DomainError {
              code: "ValidationFailed".into(),
              message: "Connection validation failed".into(),
              retryable: false,
              field: None,
              update_id: None,
            },
          ))
          .into(),
        );
      }
      value(result)
    }
    Create(file) => {
      if provider {
        let def: Definition<DnsProviderConfig> = input(file)?;
        confirm(yes, &format!("Save DNS connection {}", def.name))?;
        value(
          c.write(CreateDnsProvider {
            name: def.name,
            config: def.config,
          })
          .await?,
        )
      } else {
        let def: Definition<IngressConfig> = input(file)?;
        confirm(
          yes,
          &format!("Save ingress connection {}", def.name),
        )?;
        value(
          c.write(CreateIngress {
            name: def.name,
            config: def.config,
          })
          .await?,
        )
      }
    }
    Update { name, input: file } => {
      if provider {
        let def: Definition<DnsProviderConfig> = input(file)?;
        let current = c
          .read(GetDnsProvider {
            dns_provider: name.clone(),
          })
          .await?;
        confirm(yes, &format!("Update DNS connection {name}"))?;
        value(
          c.write(UpdateDnsProvider {
            id: current.id,
            config: def.config,
            expected_generation: def
              .expected_generation
              .unwrap_or(current.info.generation),
          })
          .await?,
        )
      } else {
        let def: Definition<IngressConfig> = input(file)?;
        let current = c
          .read(GetIngress {
            ingress: name.clone(),
          })
          .await?;
        confirm(yes, &format!("Update ingress connection {name}"))?;
        value(
          c.write(UpdateIngress {
            id: current.id,
            config: def.config,
            expected_generation: def
              .expected_generation
              .unwrap_or(current.info.generation),
          })
          .await?,
        )
      }
    }
    Delete { name } => {
      confirm(yes, &format!("Delete unused connection {name}"))?;
      if provider {
        value(c.write(DeleteDnsProvider { id: name.clone() }).await?)
      } else {
        value(c.write(DeleteIngress { id: name.clone() }).await?)
      }
    }
  }
}
async fn apply(
  reference: DomainPlan,
  yes: bool,
  wait: &WaitOptions,
) -> anyhow::Result<Value> {
  let timeout = duration(&wait.timeout)?;
  let c = client().await?;
  let server = c
    .read(GetDomainPlan {
      plan_id: reference.plan_id.clone(),
    })
    .await?;
  if reference.generation != server.generation
    || reference.domain_id != server.domain_id
    || reference.operation != server.operation
  {
    return Err(fail(
      "StalePlan",
      "Plan reference does not match the server plan",
    ));
  }
  confirm(
    yes,
    &format!(
      "Apply {:?} plan for {}: {}",
      server.operation,
      server.name,
      server.steps.join("; ")
    ),
  )?;
  let domain =
    server.domain_id.clone().unwrap_or(server.name.clone());
  let request_id = reference.request_id;
  let update = match server.operation {
    DomainPlanOperation::Remove => {
      c.execute(RemoveDomain {
        domain: domain.clone(),
        plan_id: server.plan_id,
        expected_generation: server.generation,
        request_id,
      })
      .await?
    }
    DomainPlanOperation::Migrate => {
      c.execute(MigrateDomain {
        domain: domain.clone(),
        plan_id: server.plan_id,
        expected_generation: server.generation,
        request_id,
      })
      .await?
    }
    DomainPlanOperation::FinalizeMigration => {
      c.execute(FinalizeDomainMigration {
        domain: domain.clone(),
        plan_id: server.plan_id,
        expected_generation: server.generation,
        request_id,
      })
      .await?
    }
    _ => {
      c.execute(ReconcileDomain {
        domain: domain.clone(),
        plan_id: server.plan_id,
        expected_generation: server.generation,
        request_id,
      })
      .await?
    }
  };
  let domain = match &update.target {
    komodo_client::entities::ResourceTarget::Domain(id) => id.clone(),
    _ => domain,
  };
  let generation = match server.operation {
    DomainPlanOperation::Create => 1,
    DomainPlanOperation::Update | DomainPlanOperation::Migrate => {
      server.generation + 1
    }
    _ => server.generation,
  };
  if wait.wait {
    wait_for(&domain, generation, timeout).await
  } else {
    Ok(
      json!({"domain_id":domain,"generation":generation,"state":"Accepted","update_id":update.id}),
    )
  }
}
async fn wait_for(
  domain: &str,
  generation: i64,
  timeout: Duration,
) -> anyhow::Result<Value> {
  let polling = async {
    loop {
      let status = client()
        .await?
        .read(GetDomainStatus {
          domain: domain.to_owned(),
        })
        .await?;
      if status.generation > generation {
        return Err(fail(
          "GenerationConflict",
          "Domain advanced beyond requested generation",
        ));
      }
      if status.generation == generation {
        if matches!(
          status.state,
          DomainState::Blocked | DomainState::Degraded
        ) {
          return Err(
            Failure(
              status.error.clone().unwrap_or(DomainError {
                code: "ExecutionFailed".into(),
                message:
                  "Domain did not reach Ready; inspect its conditions"
                    .into(),
                retryable: false,
                field: None,
                update_id: status.update_id.clone(),
              }),
            )
            .into(),
          );
        }
        if status.observed_generation == generation {
          if status.state == DomainState::Ready {
            return value(status);
          }
          if status.conditions.iter().any(|condition| {
            matches!(
              condition.reason.as_str(),
              "Removed" | "Abandoned"
            )
          }) {
            if let Some(id) = &status.update_id {
              let update = client()
                .await?
                .read(GetUpdate { id: id.clone() })
                .await?;
              if update.status==komodo_client::entities::update::UpdateStatus::Complete && update.success {return value(status)}
            }
          }
        }
      }
      tokio::time::sleep(Duration::from_millis(500)).await;
    }
  };
  tokio::time::timeout(timeout, polling).await.map_err(|_| {
    fail(
      "Timeout",
      "Wait timed out; the server operation may continue",
    )
  })?
}
/// Native execution commands use exactly the same server plan and apply path.
pub async fn native_execute(
  execution: &Execution,
  yes: bool,
) -> anyhow::Result<()> {
  let result = async {
  let (domain, plan_id, request_id, intent) = match execution {
    Execution::ReconcileDomain(r) => (
      &r.domain,
      &r.plan_id,
      &r.request_id,
      DomainChangeIntent::Reconcile {
        domain: r.domain.clone(),
      },
    ),
    Execution::RemoveDomain(r) => (
      &r.domain,
      &r.plan_id,
      &r.request_id,
      DomainChangeIntent::Remove {
        domain: r.domain.clone(),
      },
    ),
    Execution::MigrateDomain(r) => {
      if r.plan_id.is_empty() {
        return Err(fail(
          "InvalidInput",
          "Use km domain migrate to select candidate connections and save a plan",
        ));
      }
      (
        &r.domain,
        &r.plan_id,
        &r.request_id,
        DomainChangeIntent::Migrate {
          domain: r.domain.clone(),
          dns_provider: None,
          ingress: None,
        },
      )
    }
    Execution::FinalizeDomainMigration(r) => (
      &r.domain,
      &r.plan_id,
      &r.request_id,
      DomainChangeIntent::FinalizeMigration {
        domain: r.domain.clone(),
      },
    ),
    _ => return Err(fail("InvalidInput", "Not a domain execution")),
  };
  let mut p = if plan_id.is_empty() {
    client().await?.read(PlanDomainChange { intent }).await?
  } else {
    client()
      .await?
      .read(GetDomainPlan {
        plan_id: plan_id.clone(),
      })
      .await?
  };
  if p.domain_id.as_deref() != Some(domain) && p.name != *domain {
    return Err(fail("StalePlan", "Plan targets a different domain"));
  }
  let (expected, operation) = match execution {
    Execution::ReconcileDomain(r) => (r.expected_generation, DomainPlanOperation::Reconcile),
    Execution::RemoveDomain(r) => (r.expected_generation, DomainPlanOperation::Remove),
    Execution::MigrateDomain(r) => (r.expected_generation, DomainPlanOperation::Migrate),
    Execution::FinalizeDomainMigration(r) => (r.expected_generation, DomainPlanOperation::FinalizeMigration),
    _ => unreachable!(),
  };
  if (expected != 0 && p.generation != expected) || p.operation != operation {
    return Err(fail("StalePlan", "Plan operation or generation does not match the native command"));
  }
  p.request_id = if request_id.is_empty() {
    uuid::Uuid::new_v4().to_string()
  } else {
    request_id.clone()
  };
  apply(
    p,
    yes,
    &WaitOptions {
      wait: false,
      timeout: "120s".into(),
    },
  )
  .await
  }.await;
  match result {
    Ok(v) => {
      println!("{}", serde_json::to_string_pretty(&v)?);
      Ok(())
    }
    Err(e) => {
      let e = error(&e);
      eprintln!("{}: {}", e.code, e.message);
      std::process::exit(exit_code(&e.code))
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use clap::Parser;
  use komodo_client::entities::config::cli::args::CliArgs;
  #[test]
  fn grammar_workflows() {
    for argv in [
      vec!["km", "execute", "reconcile-domain", "app", "--yes"],
      vec!["km", "list", "domains", "--format", "json"],
      vec![
        "km",
        "domain",
        "plan",
        "--file",
        "app.toml",
        "--out",
        "app.plan.json",
      ],
      vec![
        "km",
        "domain",
        "apply",
        "--plan",
        "app.plan.json",
        "--yes",
        "--wait",
        "--format",
        "json",
      ],
      vec![
        "km",
        "domain",
        "migrate",
        "app",
        "--ingress",
        "tunnel",
        "--out",
        "move.json",
      ],
      vec![
        "km",
        "domain",
        "provider",
        "create",
        "--file",
        "-",
        "--input-format",
        "json",
        "--yes",
      ],
      vec![
        "km",
        "domain",
        "update",
        "app",
        "--file",
        "app.toml",
        "--expected-generation",
        "2",
      ],
      vec!["km", "domain", "remove", "app"],
      vec!["km", "domain", "export", "app", "--format", "toml"],
    ] {
      CliArgs::try_parse_from(argv).unwrap();
    }
    assert!(
      CliArgs::try_parse_from(["km", "domain", "migrate", "app"])
        .is_err()
    );
    assert!(
      CliArgs::try_parse_from(["km", "domain", "plan"]).is_err()
    );
  }
  #[test]
  fn schema_and_stdin_format() {
    assert!(
      input::<Value>(&FileInput {
        file: "-".into(),
        input_format: None
      })
      .is_err()
    );
    assert!(
      parse_input::<Definition<DomainConfig>>(
        "[]",
        InputFormat::Json
      )
      .is_err()
    );
    assert!(
      parse_input::<Definition<DomainConfig>>(
        r#"{"name":"x","config":{},"token":"secret"}"#,
        InputFormat::Json
      )
      .is_err()
    );
  }
  #[test]
  fn stable_errors_and_json() {
    for (code, exit) in [
      ("UnsupportedCapability", 2),
      ("StalePlan", 3),
      ("GenerationConflict", 3),
      ("Drifted", 3),
      ("Forbidden", 4),
      ("Timeout", 5),
    ] {
      assert_eq!(exit_code(code), exit);
    }
    let value = envelope(json!({"state":"Accepted"}), None);
    assert_eq!(value["api_version"], "domain.v1");
    assert_eq!(value["ok"], true);
  }
  #[test]
  fn timeout_validation() {
    assert_eq!(duration("120s").unwrap(), Duration::from_secs(120));
    assert!(duration("0s").is_err());
    assert!(duration("forever").is_err());
  }
  #[test]
  fn noninteractive_requires_yes() {
    if !std::io::stdin().is_terminal() {
      assert!(confirm(false, "test").is_err());
    }
    assert!(confirm(true, "test").is_ok());
  }
}
