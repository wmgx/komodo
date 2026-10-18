//! Real loopback HTTP fixtures exercising the production reqwest code path.
//! No Cloudflare endpoints, accounts, or credentials are contacted.
use std::{
  sync::{Arc, Mutex},
  time::Duration,
};

use reqwest::Client;
use serde_json::{Value, json};

use super::CloudflareClient;
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::TcpListener,
  task::JoinHandle,
};

#[allow(dead_code)]
struct Request {
  method: String,
  path: String,
  body: Value,
}
struct Response {
  status: u16,
  body: Value,
  delay: Duration,
}
impl Response {
  fn ok(result: Value) -> Self {
    Self {
      status: 200,
      body: json!({"success":true,"result":result}),
      delay: Duration::ZERO,
    }
  }
  fn error(status: u16) -> Self {
    Self {
      status,
      body: json!({"success":false,"errors":[{"message":"fixture-secret must never leak"}]}),
      delay: Duration::ZERO,
    }
  }
}
struct Fixture {
  base: String,
  task: JoinHandle<()>,
}
impl Drop for Fixture {
  fn drop(&mut self) {
    self.task.abort();
  }
}
impl Fixture {
  async fn start(
    handler: impl Fn(Request) -> Response + Send + Sync + 'static,
  ) -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handler = Arc::new(handler);
    let task = tokio::spawn(async move {
      while let Ok((mut stream, _)) = listener.accept().await {
        let handler = handler.clone();
        tokio::spawn(async move {
          let mut bytes = Vec::new();
          let mut buf = [0u8; 4096];
          let header_end = loop {
            let n = stream.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
              return;
            }
            bytes.extend_from_slice(&buf[..n]);
            if let Some(pos) =
              bytes.windows(4).position(|s| s == b"\r\n\r\n")
            {
              break pos + 4;
            }
            assert!(
              bytes.len() < 65536,
              "unexpected fixture request size"
            );
          };
          let headers = String::from_utf8_lossy(&bytes[..header_end])
            .into_owned();
          let length = headers
            .lines()
            .find_map(|line| {
              let (name, value) = line.split_once(':')?;
              name
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
          while bytes.len() < header_end + length {
            let n = stream.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
              return;
            }
            bytes.extend_from_slice(&buf[..n]);
          }
          let mut first =
            headers.lines().next().unwrap().split_whitespace();
          let request = Request {
            method: first.next().unwrap().into(),
            path: first.next().unwrap().into(),
            body: if length == 0 {
              Value::Null
            } else {
              serde_json::from_slice(
                &bytes[header_end..header_end + length],
              )
              .unwrap()
            },
          };
          let response = handler(request);
          if !response.delay.is_zero() {
            tokio::time::sleep(response.delay).await;
          }
          let body = response.body.to_string();
          let head = format!(
            "HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 0\r\nConnection: close\r\n\r\n",
            response.status,
            body.len()
          );
          let _ = stream.write_all(head.as_bytes()).await;
          let _ = stream.write_all(body.as_bytes()).await;
        });
      }
    });
    Self { base, task }
  }
  fn client(&self, timeout: Duration) -> CloudflareClient {
    CloudflareClient {
      client: Client::builder()
        .no_proxy()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap(),
      token: "fixture-secret".into(),
      base_url: Some(self.base.clone()),
    }
  }
}

#[tokio::test]
async fn dns_http_paginates_and_filters_exact_hostname() {
  let paths = Arc::new(Mutex::new(Vec::new()));
  let seen = paths.clone();
  let fixture = Fixture::start(move |request| {
    assert_eq!(request.method, "GET");
    seen.lock().unwrap().push(request.path.clone());
    let mut response = if request.path.contains("page=1&") {
      Response::ok(json!(
        (0..100)
          .map(|i| json!({
            "id": format!("record{i}"),
            "name": "app.example.com",
            "type": "A",
            "content": "192.0.2.1"
          }))
          .collect::<Vec<_>>()
      ))
    } else {
      Response::ok(json!([
        { "id": "last", "name": "app.example.com", "type": "A" },
        { "id": "foreign", "name": "other.example.com", "type": "A" }
      ]))
    };
    response.body["result_info"] = json!({ "total_pages": 2 });
    response
  })
  .await;
  let records = fixture
    .client(Duration::from_secs(2))
    .list_dns("zone", "app.example.com")
    .await
    .unwrap();
  assert_eq!(records.len(), 101);
  assert!(records.iter().all(|r| r["name"] == "app.example.com"));
  let paths = paths.lock().unwrap();
  assert_eq!(paths.len(), 2);
  assert!(paths[1].contains("page=2&"));
}

#[tokio::test]
async fn dns_http_retries_get_and_sanitizes_authorization_errors() {
  let count = Arc::new(Mutex::new(0u32));
  let seen = count.clone();
  let fixture = Fixture::start(move |request| {
    if request.path.contains("denied.example.com") {
      return Response::error(403);
    }
    let mut count = seen.lock().unwrap();
    *count += 1;
    if *count == 1 {
      Response::error(429)
    } else {
      Response::ok(json!([]))
    }
  })
  .await;
  let client = fixture.client(Duration::from_secs(2));
  assert!(
    client
      .list_dns("zone", "ok.example.com")
      .await
      .unwrap()
      .is_empty()
  );
  assert_eq!(*count.lock().unwrap(), 2);
  let error = client
    .list_dns("zone", "denied.example.com")
    .await
    .unwrap_err();
  assert_eq!(error.code, "AuthFailed");
  assert!(!error.retryable);
  let rendered = format!("{error} {error:?}");
  assert!(!rendered.contains("fixture-secret"));
}

#[tokio::test]
async fn discover_lists_zones_tunnels_and_warns_on_missing_permission()
 {
  let fixture = Fixture::start(|request| {
    assert_eq!(request.method, "GET");
    let path = request.path.as_str();
    if path.starts_with("/zones") {
      Response::ok(json!([{ "id": "z1", "name": "Example.com" }]))
    } else if path.starts_with("/accounts?") {
      Response::ok(json!([
        { "id": "acc1", "name": "Main" },
        { "id": "acc2", "name": "Other" }
      ]))
    } else if path
      .starts_with("/accounts/acc1/cfd_tunnel?is_deleted=false")
    {
      Response::ok(json!([
        { "id": "t1", "name": "home", "status": "healthy" }
      ]))
    } else {
      Response::error(403)
    }
  })
  .await;
  let found = super::super::cloudflare::discover_with(
    &fixture.client(Duration::from_secs(2)),
  )
  .await
  .unwrap();
  assert_eq!(found.zones, vec!["example.com"]);
  assert_eq!(found.ingresses.len(), 1);
  assert_eq!(found.ingresses[0].id, "t1");
  assert_eq!(found.ingresses[0].account_id.as_deref(), Some("acc1"));
  assert_eq!(found.ingresses[0].status.as_deref(), Some("healthy"));
  assert_eq!(found.warnings.len(), 1);
  assert!(found.warnings[0].contains("Other"));
  assert!(!format!("{found:?}").contains("fixture-secret"));
}

fn provision_with(
  fixture: &Fixture,
) -> crate::domain::provider::cloudflare_provision::CloudflareProvision
{
  crate::domain::provider::cloudflare_provision::CloudflareProvision::new(
    Arc::new(fixture.client(Duration::from_secs(2))),
    Default::default(),
  )
}

#[tokio::test]
async fn provision_creates_tunnel_inits_config_and_fetches_token() {
  use crate::domain::provider::ProvisionProvider;

  let seen =
    Arc::new(Mutex::new(Vec::<(String, String, Value)>::new()));
  let log = seen.clone();
  let fixture = Fixture::start(move |request| {
    log.lock().unwrap().push((
      request.method.clone(),
      request.path.clone(),
      request.body.clone(),
    ));
    match (request.method.as_str(), request.path.as_str()) {
      ("GET", p) if p.starts_with("/accounts?") => {
        Response::ok(json!([{ "id": "acc1", "name": "me" }]))
      }
      ("POST", "/accounts/acc1/cfd_tunnel") => {
        Response::ok(json!({ "id": "tun1" }))
      }
      ("PUT", "/accounts/acc1/cfd_tunnel/tun1/configurations") => {
        Response::ok(json!({}))
      }
      ("GET", "/accounts/acc1/cfd_tunnel/tun1/token") => {
        Response::ok(json!("connector-token"))
      }
      _ => Response::error(404),
    }
  })
  .await;
  let done = provision_with(&fixture)
    .provision("komodo-home", "mirror.example/cloudflared:latest")
    .await
    .unwrap();
  assert_eq!(done.handle.id, "tun1");
  assert_eq!(done.handle.scope, "acc1");
  assert!(done.handle.managed);
  assert_eq!(
    done.connector.image,
    "mirror.example/cloudflared:latest"
  );
  assert_eq!(done.connector.command, "tunnel --no-autoupdate run");
  assert_eq!(
    done.connector.env,
    vec![("TUNNEL_TOKEN".to_string(), "connector-token".to_string())]
  );
  let seen = seen.lock().unwrap();
  let create = seen
    .iter()
    .find(|(m, p, _)| m == "POST" && p.ends_with("/cfd_tunnel"))
    .unwrap();
  assert_eq!(
    create.2,
    json!({ "name": "komodo-home", "config_src": "cloudflare" })
  );
  let put = seen.iter().find(|(m, _, _)| m == "PUT").unwrap();
  assert_eq!(
    put.2,
    json!({ "config": { "ingress": [{ "service": "http_status:404" }] } })
  );
}

#[tokio::test]
async fn provision_rolls_back_the_tunnel_when_token_fetch_fails() {
  use crate::domain::provider::ProvisionProvider;

  let seen = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
  let log = seen.clone();
  let fixture = Fixture::start(move |request| {
    log
      .lock()
      .unwrap()
      .push((request.method.clone(), request.path.clone()));
    match (request.method.as_str(), request.path.as_str()) {
      ("GET", p) if p.starts_with("/accounts?") => {
        Response::ok(json!([{ "id": "acc1", "name": "me" }]))
      }
      ("POST", "/accounts/acc1/cfd_tunnel") => {
        Response::ok(json!({ "id": "tun1" }))
      }
      ("PUT", _) => Response::ok(json!({})),
      ("GET", p) if p.ends_with("/token") => Response::error(403),
      ("DELETE", _) => Response::ok(json!({})),
      _ => Response::error(404),
    }
  })
  .await;
  let error = provision_with(&fixture)
    .provision("komodo-home", "img")
    .await
    .err()
    .unwrap();
  assert_eq!(error.code, "AuthFailed");
  assert!(!format!("{error:?}").contains("fixture-secret"));
  let seen = seen.lock().unwrap();
  assert!(seen.iter().any(|(m, p)| {
    m == "DELETE" && p == "/accounts/acc1/cfd_tunnel/tun1"
  }));
}
