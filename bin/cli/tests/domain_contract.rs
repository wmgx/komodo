//! HTTP contract fixtures exercise the real `km` process. They do not validate
//! provider integration, ownership enforcement, or production readiness.
use komodo_client::entities::{domain::*, update::Update};
use serde_json::{Value, json};
use std::{
  io::{Read, Write},
  net::TcpListener,
  path::PathBuf,
  process::{Command, Output, Stdio},
  sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
  },
  thread,
  time::Duration,
};
struct Api {
  url: String,
  requests: Arc<Mutex<Vec<Value>>>,
  stop: Arc<AtomicBool>,
  thread: Option<thread::JoinHandle<()>>,
  dir: PathBuf,
}
impl Api {
  fn new(reply: impl Fn(&Value) -> Value + Send + 'static) -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let thread = thread::spawn(move || {
      'connections: while !stopped.load(Ordering::Relaxed) {
        let Ok((mut socket, _)) = listener.accept() else {
          thread::sleep(Duration::from_millis(5));
          continue;
        };
        socket
          .set_read_timeout(Some(Duration::from_secs(3)))
          .unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0u8; 8192];
        let request = loop {
          let n = match socket.read(&mut buf) {
            // A client timeout may close a connection before sending a
            // complete request. Only complete requests enter the assertions.
            Ok(0) => continue 'connections,
            Ok(n) => n,
            Err(error) if expected_disconnect(&error) => {
              continue 'connections;
            }
            Err(error) => {
              panic!("HTTP fixture request read failed: {error}")
            }
          };
          bytes.extend_from_slice(&buf[..n]);
          if let Some(end) =
            bytes.windows(4).position(|w| w == b"\r\n\r\n")
          {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
              .lines()
              .find_map(|line| {
                line
                  .to_lowercase()
                  .strip_prefix("content-length:")
                  .map(|s| s.trim().parse::<usize>().unwrap())
              })
              .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
              break serde_json::from_slice::<Value>(
                &bytes[end + 4..end + 4 + length],
              )
              .unwrap();
            }
          }
        };
        captured.lock().unwrap().push(request.clone());
        let body = if request["type"] == "GetVersion" {
          json!({"version":"2.3.3"})
        } else {
          reply(&request)
        }
        .to_string();
        if let Err(error) = write!(
          socket,
          "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
          body.len(),
          body
        ) {
          // The request assertions above still run when timeout cancels the
          // client while the fixture is preparing its response.
          assert!(
            expected_disconnect(&error),
            "HTTP fixture response write failed: {error}"
          );
        }
      }
    });
    let dir = std::env::temp_dir()
      .join(format!("komodo-cli-contract-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    Self {
      url,
      requests,
      stop,
      thread: Some(thread),
      dir,
    }
  }
  fn run(&self, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_km"))
      .args(args)
      .env("KOMODO_CLI_HOST", &self.url)
      .env("KOMODO_CLI_KEY", "contract-key")
      .env("KOMODO_CLI_SECRET", "credential-must-not-leak")
      .env("KOMODO_CLI_CONFIG_PATHS", self.dir.to_str().unwrap())
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .spawn()
      .unwrap();
    if let Some(input) = stdin {
      child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    } else {
      drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
  }
}
impl Drop for Api {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
    self.thread.take().unwrap().join().unwrap();
    let _ = std::fs::remove_dir_all(&self.dir);
  }
}
fn output(out: Output, exit: i32) -> Value {
  assert_eq!(
    out.status.code(),
    Some(exit),
    "stderr: {} stdout: {}",
    String::from_utf8_lossy(&out.stderr),
    String::from_utf8_lossy(&out.stdout)
  );
  assert!(
    !String::from_utf8_lossy(&out.stdout)
      .contains("credential-must-not-leak")
  );
  let value: Value = serde_json::from_slice(&out.stdout).unwrap();
  assert_eq!(value["api_version"], "domain.v1");
  value
}
fn config() -> Value {
  json!({"hostname":"app.example.com","target":{"type":"Url","params":{"url":"http://192.0.2.1:8080"}},"dns":{"type":"External"},"ingress":"edge"})
}
fn plan(operation: DomainPlanOperation) -> DomainPlan {
  DomainPlan {
    plan_id: "plan-1".into(),
    domain_id: Some("012345678901234567890123".into()),
    name: "app".into(),
    generation: 2,
    expires_at: 9999999999999,
    operation,
    steps: vec!["Apply owned objects".into()],
    warnings: vec![],
    requires_downtime: false,
    request_id: String::new(),
  }
}
#[test]
fn stdin_create_and_noninteractive_confirmation() {
  let api = Api::new(|r| {
    assert_eq!(r["type"], "CreateDomain");
    let mut domain = Domain::default();
    domain.name = "app".into();
    domain.config =
      serde_json::from_value(r["params"]["config"].clone()).unwrap();
    serde_json::to_value(domain).unwrap()
  });
  let input = json!({"name":"app","config":config()}).to_string();
  let rejected = output(
    api.run(
      &[
        "domain",
        "create",
        "--file",
        "-",
        "--input-format",
        "json",
        "--format",
        "json",
      ],
      Some(&input),
    ),
    1,
  );
  assert_eq!(rejected["error"]["code"], "ConfirmationRequired");
  assert!(api.requests.lock().unwrap().is_empty());
  let accepted = output(
    api.run(
      &[
        "domain",
        "create",
        "--file",
        "-",
        "--input-format",
        "json",
        "--format",
        "json",
        "--yes",
      ],
      Some(&input),
    ),
    0,
  );
  assert_eq!(accepted["data"]["name"], "app");
}
#[test]
fn cli_argument_errors_are_json() {
  let api = Api::new(|_| panic!("no request expected"));
  let v = output(
    api.run(&["domain", "migrate", "app", "--format", "json"], None),
    2,
  );
  assert_eq!(v["error"]["code"], "InvalidInput");
  let v = output(
    api.run(
      &[
        "domain", "create", "--file", "-", "--format", "json",
        "--yes",
      ],
      Some("{}"),
    ),
    2,
  );
  assert_eq!(v["error"]["code"], "InvalidInput");
}
#[test]
fn apply_reuses_request_id_and_reports_accepted() {
  let p = plan(DomainPlanOperation::Reconcile);
  let server = p.clone();
  let api = Api::new(move |r| match r["type"].as_str().unwrap() {
    "GetDomainPlan" => serde_json::to_value(&server).unwrap(),
    "ReconcileDomain" => {
      let mut u = Update::default();
      u.id = "012345678901234567890124".into();
      serde_json::to_value(u).unwrap()
    }
    _ => panic!("unexpected request"),
  });
  let path = api.dir.join("plan.json");
  std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
  for _ in 0..2 {
    let v = output(
      api.run(
        &[
          "domain",
          "apply",
          "--plan",
          path.to_str().unwrap(),
          "--yes",
          "--format",
          "json",
        ],
        None,
      ),
      0,
    );
    assert_eq!(v["data"]["state"], "Accepted");
  }
  let requests = api.requests.lock().unwrap();
  let ids: Vec<_> = requests
    .iter()
    .filter(|r| r["type"] == "ReconcileDomain")
    .map(|r| r["params"]["request_id"].clone())
    .collect();
  assert_eq!(ids.len(), 2);
  assert_eq!(ids[0], ids[1]);
  assert!(!ids[0].as_str().unwrap().is_empty());
}
#[test]
fn stale_local_plan_never_executes() {
  let mut p = plan(DomainPlanOperation::Reconcile);
  let server = p.clone();
  let api = Api::new(move |r| {
    assert_eq!(r["type"], "GetDomainPlan");
    serde_json::to_value(&server).unwrap()
  });
  p.generation = 1;
  let path = api.dir.join("plan.json");
  std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
  let v = output(
    api.run(
      &[
        "domain",
        "apply",
        "--plan",
        path.to_str().unwrap(),
        "--yes",
        "--format",
        "json",
      ],
      None,
    ),
    3,
  );
  assert_eq!(v["error"]["code"], "StalePlan");
}
#[test]
fn wait_timeout_is_not_cancellation() {
  let api = Api::new(|r| {
    assert_eq!(r["type"], "GetDomainStatus");
    json!({"domain_id":"app","generation":2,"observed_generation":1,"state":"Applying","conditions":[],"update_id":"operation","error":null})
  });
  let v = output(
    api.run(
      &[
        "domain",
        "wait",
        "app",
        "--generation",
        "2",
        "--timeout",
        "50ms",
        "--format",
        "json",
      ],
      None,
    ),
    5,
  );
  assert_eq!(v["error"]["code"], "Timeout");
}
#[test]
fn migration_and_remove_are_read_only_plans() {
  let api = Api::new(|r| {
    assert_eq!(r["type"], "PlanDomainChange");
    serde_json::to_value(plan(DomainPlanOperation::Migrate)).unwrap()
  });
  for args in [
    vec![
      "domain",
      "migrate",
      "app",
      "--ingress",
      "tunnel",
      "--format",
      "json",
    ],
    vec!["domain", "remove", "app", "--format", "json"],
  ] {
    output(api.run(&args, None), 0);
  }
  assert!(api.requests.lock().unwrap().iter().all(|r| matches!(
    r["type"].as_str(),
    Some("GetVersion" | "PlanDomainChange")
  )));
}
#[test]
fn update_carries_expected_generation() {
  let api = Api::new(|r| {
    assert_eq!(r["type"], "UpdateDomain");
    assert_eq!(r["params"]["expected_generation"], 7);
    let mut d = Domain::default();
    d.name = "app".into();
    d.config =
      serde_json::from_value(r["params"]["config"].clone()).unwrap();
    serde_json::to_value(d).unwrap()
  });
  let v = output(
    api.run(
      &[
        "domain",
        "update",
        "app",
        "--file",
        "-",
        "--input-format",
        "json",
        "--expected-generation",
        "7",
        "--yes",
        "--format",
        "json",
      ],
      Some(&json!({"name":"app","config":config()}).to_string()),
    ),
    0,
  );
  assert!(v["ok"].as_bool().unwrap());
}
#[test]
fn export_contains_only_desired_config_and_names() {
  let api = Api::new(|r| match r["type"].as_str().unwrap() {
    "GetDomain" => {
      let mut d = Domain::default();
      d.id = "012345678901234567890123".into();
      d.name = "app".into();
      d.config = serde_json::from_value(config()).unwrap();
      d.config.ingress = "012345678901234567890124".into();
      d.info.last_operation_id = Some("internal-operation".into());
      serde_json::to_value(d).unwrap()
    }
    "GetIngress" => {
      let mut ingress = Ingress::default();
      ingress.name = "edge".into();
      serde_json::to_value(ingress).unwrap()
    }
    _ => panic!("unexpected request"),
  });
  let out =
    api.run(&["domain", "export", "app", "--format", "toml"], None);
  assert!(
    out.status.success(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );
  let text = String::from_utf8(out.stdout).unwrap();
  let v: toml::Value = toml::from_str(&text).unwrap();
  assert_eq!(v["config"]["ingress"].as_str(), Some("edge"));
  assert!(!text.contains("internal-operation"));
  assert!(!text.contains("credential-must-not-leak"));
}
#[test]
fn capability_error_preserves_machine_code() {
  let api = Api::new(|r| {
    assert_eq!(r["type"], "GetDomainStatus");
    json!({"domain_id":"app","generation":2,"observed_generation":2,"state":"Blocked","conditions":[],"update_id":"operation","error":{"code":"UnsupportedCapability","message":"TLS profile unsupported","retryable":false,"field":"tls.profile","update_id":"operation"}})
  });
  let v = output(
    api.run(
      &[
        "domain",
        "wait",
        "app",
        "--generation",
        "2",
        "--format",
        "json",
      ],
      None,
    ),
    2,
  );
  assert_eq!(v["error"]["code"], "UnsupportedCapability");
  assert_eq!(v["error"]["field"], "tls.profile");
}
#[test]
fn toml_stdin_plan_uses_shared_config_schema() {
  let api = Api::new(|request| {
    assert_eq!(request["type"], "PlanDomainChange");
    assert_eq!(request["params"]["intent"]["type"], "Create");
    assert_eq!(
      request["params"]["intent"]["params"]["config"]["hostname"],
      "app.example.com"
    );
    serde_json::to_value(plan(DomainPlanOperation::Create)).unwrap()
  });
  let input =
    toml::to_string(&json!({"name":"app","config":config()}))
      .unwrap();
  let result = output(
    api.run(
      &[
        "domain",
        "plan",
        "--file",
        "-",
        "--input-format",
        "toml",
        "--format",
        "json",
      ],
      Some(&input),
    ),
    0,
  );
  assert_eq!(result["data"]["operation"], "Create");
}

// A timed-out HTTP client is allowed to drop its socket. Other I/O failures
// still fail the fixture rather than hiding server or test errors.
fn expected_disconnect(error: &std::io::Error) -> bool {
  matches!(
    error.kind(),
    std::io::ErrorKind::BrokenPipe
      | std::io::ErrorKind::ConnectionReset
      | std::io::ErrorKind::ConnectionAborted
  )
}
