//! Real loopback HTTP fixtures exercising the production reqwest code path.
//! No Cloudflare endpoints, accounts, or credentials are contacted.
use super::*;
use std::sync::{Arc, Mutex};
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::TcpListener,
  task::JoinHandle,
};

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
  let fixture=Fixture::start(move|request|{
  assert_eq!(request.method,"GET");seen.lock().unwrap().push(request.path.clone());
  let mut response=if request.path.contains("page=1&"){
   Response::ok(json!((0..100).map(|i|json!({"id":format!("record{i}"),"name":"app.example.com","type":"A","content":"192.0.2.1"})).collect::<Vec<_>>()))
  }else{Response::ok(json!([{"id":"last","name":"app.example.com","type":"A"},{"id":"foreign","name":"other.example.com","type":"A"}]))};
  response.body["result_info"]=json!({"total_pages":2});response
 }).await;
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
async fn dns_http_retries_rate_limit_and_sanitizes_authorization_errors()
 {
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
  assert_eq!(error.code, "PermissionDenied");
  assert!(!error.retryable);
  assert!(
    !serde_json::to_string(&error)
      .unwrap()
      .contains("fixture-secret")
  );
}

#[tokio::test]
async fn dns_http_recovers_timeout_success_without_duplicate_and_blocks_drift()
 {
  let record = Arc::new(Mutex::new(None::<Value>));
  let stored = record.clone();
  let writes = Arc::new(Mutex::new(Vec::new()));
  let mutations = writes.clone();
  let fixture = Fixture::start(move |request| {
    if request.method == "POST" {
      mutations.lock().unwrap().push(request.method);
      let mut value = request.body;
      value["id"] = json!("record");
      value["tags"] = json!([]);
      value["settings"] = json!({});
      *stored.lock().unwrap() = Some(value.clone());
      let mut response = Response::ok(value);
      response.delay = Duration::from_millis(1500);
      return response;
    }
    assert_eq!(
      request.method, "GET",
      "drift and retry must not issue PUT/DELETE"
    );
    if request.path.contains("/dns_records?") {
      Response::ok(json!(
        stored.lock().unwrap().iter().cloned().collect::<Vec<_>>()
      ))
    } else {
      Response::ok(stored.lock().unwrap().clone().unwrap())
    }
  })
  .await;
  let client = fixture.client(Duration::from_millis(500));
  let intent = DnsIntent {
    hostname: "app.example.com".into(),
    record_type: "A".into(),
    content: "192.0.2.1".into(),
    ttl: 60,
    proxied: false,
  };
  let error = client
    .apply_dns("zone", &intent, None, "komodo:instance:domain")
    .await
    .unwrap_err();
  assert!(error.retryable);
  let handle = client
    .apply_dns("zone", &intent, None, "komodo:instance:domain")
    .await
    .unwrap();
  let unchanged = client
    .apply_dns(
      "zone",
      &intent,
      Some(&handle),
      "komodo:instance:domain",
    )
    .await
    .unwrap();
  assert_eq!(unchanged, handle);
  assert_eq!(writes.lock().unwrap().as_slice(), ["POST"]);
  record.lock().unwrap().as_mut().unwrap()["content"] =
    json!("192.0.2.99");
  assert_eq!(
    client
      .apply_dns(
        "zone",
        &intent,
        Some(&handle),
        "komodo:instance:domain"
      )
      .await
      .unwrap_err()
      .code,
    "Drifted"
  );
  assert_eq!(
    client.remove_dns("zone", &handle).await.unwrap_err().code,
    "Drifted"
  );
  record.lock().unwrap().as_mut().unwrap()["comment"] =
    json!("manual");
  assert_eq!(
    client
      .apply_dns("zone", &intent, None, "komodo:instance:domain")
      .await
      .unwrap_err()
      .code,
    "OwnershipConflict"
  );
  assert_eq!(writes.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn tunnel_http_preserves_full_configuration_and_avoids_redundant_put()
 {
  let initial = json!({"future":{"preserve":true},"originRequest":{"connectTimeout":10},"ingress":[{"hostname":"manual.example.com","service":"http://manual","future":true},{"service":"http_status:404"}]});
  let config = Arc::new(Mutex::new(initial.clone()));
  let stored = config.clone();
  let writes = Arc::new(Mutex::new(0usize));
  let mutations = writes.clone();
  let fixture = Fixture::start(move |request| {
    assert!(request.path.ends_with("/configurations"));
    if request.method == "PUT" {
      *mutations.lock().unwrap() += 1;
      *stored.lock().unwrap() = request.body["config"].clone();
    }
    Response::ok(json!({"config":stored.lock().unwrap().clone()}))
  })
  .await;
  let client = fixture.client(Duration::from_secs(2));
  let route = RouteSpec {
    binding_id: "binding".into(),
    hostname: "app.example.com".into(),
    backend_url: "http://app:8080".into(),
  };
  let handle = client
    .apply_tunnel_route(
      "account",
      "tunnel",
      &fingerprint(&initial),
      &route,
      None,
    )
    .await
    .unwrap();
  let next =
    client.observe_tunnel("account", "tunnel").await.unwrap();
  assert_eq!(next["future"], initial["future"]);
  assert_eq!(next["ingress"][0], initial["ingress"][0]);
  assert_eq!(next["ingress"][2], initial["ingress"][1]);
  client
    .apply_tunnel_route(
      "account",
      "tunnel",
      &fingerprint(&next),
      &route,
      Some(&handle),
    )
    .await
    .unwrap();
  assert_eq!(*writes.lock().unwrap(), 1);
  config.lock().unwrap()["future"] = json!({"manual_change":true});
  assert_eq!(
    client
      .apply_tunnel_route(
        "account",
        "tunnel",
        &fingerprint(&next),
        &route,
        Some(&handle)
      )
      .await
      .unwrap_err()
      .code,
    "Drifted"
  );
  assert_eq!(*writes.lock().unwrap(), 1);
}
