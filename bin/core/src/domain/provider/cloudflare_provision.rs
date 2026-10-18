//! One-click Cloudflare Tunnel: create a remote-managed tunnel, hand out
//! the connector token, and keep the account params in sync.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::domain::error::ProviderError;

use super::{
  BoxFut, ServerRef,
  cloudflare::{CloudflareParams, TunnelEntry},
  cloudflare_client::CloudflareClient,
  provision::{
    ConnectorSpec, ProvisionProvider, Provisioned, TunnelHandle,
  },
};

const DEFAULT_IMAGE: &str = "cloudflare/cloudflared:latest";
const COMMAND: &str = "tunnel --no-autoupdate run";

pub(super) struct CloudflareProvision {
  client: Arc<CloudflareClient>,
  params: CloudflareParams,
}

impl CloudflareProvision {
  pub(super) fn new(
    client: Arc<CloudflareClient>,
    params: CloudflareParams,
  ) -> Self {
    Self { client, params }
  }

  /// The Cloudflare account to create tunnels in: the one of tunnels
  /// already registered, else the only account the token sees.
  async fn account_id(&self) -> Result<String, ProviderError> {
    if let Some(t) = self.params.tunnels.first() {
      return Ok(t.account_id.clone());
    }
    let mut accounts = self.client.list_accounts().await?;
    match accounts.len() {
      1 => Ok(accounts.remove(0).0),
      0 => Err(ProviderError::invalid(
        "token 看不到任何 Cloudflare 账号，请检查 token 权限（账户 - Cloudflare Tunnel - 编辑）",
      )),
      _ => Err(ProviderError::invalid(format!(
        "token 能访问多个 Cloudflare 账号（{}），请先在账号参数 tunnels 里登记一条已有隧道以确定用哪个",
        accounts
          .iter()
          .map(|(_, n)| n.as_str())
          .collect::<Vec<_>>()
          .join(", ")
      ))),
    }
  }

  fn spec(&self, image: &str, token: String) -> ConnectorSpec {
    ConnectorSpec {
      image: image.to_string(),
      command: COMMAND.to_string(),
      env: vec![("TUNNEL_TOKEN".to_string(), token)],
    }
  }
}

fn handle_of(t: &TunnelEntry) -> TunnelHandle {
  TunnelHandle {
    name: t.name.clone(),
    scope: t.account_id.clone(),
    id: t.tunnel_id.clone(),
    managed: t.managed,
  }
}

fn tunnels_of(raw: &Value) -> Vec<Value> {
  raw["tunnels"].as_array().cloned().unwrap_or_default()
}

fn with_field(raw: &Value, key: &str, value: Value) -> Value {
  let mut out = if raw.is_object() {
    raw.clone()
  } else {
    json!({})
  };
  out[key] = value;
  out
}

impl ProvisionProvider for CloudflareProvision {
  fn default_image(&self) -> String {
    self
      .params
      .cloudflared_image
      .clone()
      .unwrap_or_else(|| DEFAULT_IMAGE.to_string())
  }

  fn registered(&self, server: &ServerRef) -> Option<TunnelHandle> {
    self
      .params
      .tunnels
      .iter()
      .find(|t| {
        t.server
          .as_deref()
          .is_some_and(|s| s == server.id || s == server.name)
      })
      .map(handle_of)
  }

  fn provision<'a>(
    &'a self,
    name: &'a str,
    image: &'a str,
  ) -> BoxFut<'a, Provisioned> {
    Box::pin(async move {
      let account = self.account_id().await?;
      let id = self.client.create_tunnel(&account, name).await?;
      let handle = TunnelHandle {
        name: name.to_string(),
        scope: account.clone(),
        id: id.clone(),
        managed: true,
      };
      let rest = async {
        // Only a catch-all: Domains add their own rules later.
        self
          .client
          .put_tunnel_config(
            &account,
            &id,
            &json!({ "ingress": [{ "service": "http_status:404" }] }),
          )
          .await?;
        self.client.tunnel_token(&account, &id).await
      }
      .await;
      match rest {
        Ok(token) => Ok(Provisioned {
          connector: self.spec(image, token),
          handle,
        }),
        Err(e) => {
          // Don't leave a tunnel nobody knows about.
          if let Err(cleanup) =
            self.client.delete_tunnel(&account, &id).await
          {
            cleanup.log("rollback created tunnel");
          }
          Err(e)
        }
      }
    })
  }

  fn connector<'a>(
    &'a self,
    handle: &'a TunnelHandle,
    image: &'a str,
  ) -> BoxFut<'a, ConnectorSpec> {
    Box::pin(async move {
      let token =
        self.client.tunnel_token(&handle.scope, &handle.id).await?;
      Ok(self.spec(image, token))
    })
  }

  fn status<'a>(
    &'a self,
    handle: &'a TunnelHandle,
  ) -> BoxFut<'a, String> {
    Box::pin(async move {
      self.client.tunnel_status(&handle.scope, &handle.id).await
    })
  }

  fn destroy<'a>(
    &'a self,
    handle: &'a TunnelHandle,
  ) -> BoxFut<'a, ()> {
    Box::pin(async move {
      self.client.delete_tunnel(&handle.scope, &handle.id).await
    })
  }

  fn register(
    &self,
    params: &Value,
    handle: &TunnelHandle,
    server: &ServerRef,
    image: &str,
  ) -> Value {
    let mut tunnels = tunnels_of(params);
    tunnels.push(json!({
      "name": handle.name,
      "account_id": handle.scope,
      "tunnel_id": handle.id,
      "server": server.name,
      "managed": true,
    }));
    let out = with_field(params, "tunnels", Value::Array(tunnels));
    with_field(&out, "cloudflared_image", json!(image))
  }

  fn unregister(
    &self,
    params: &Value,
    handle: &TunnelHandle,
  ) -> Value {
    let tunnels: Vec<Value> = tunnels_of(params)
      .into_iter()
      .filter(|t| t["tunnel_id"].as_str() != Some(handle.id.as_str()))
      .collect();
    with_field(params, "tunnels", Value::Array(tunnels))
  }

  fn remember_image(&self, params: &Value, image: &str) -> Value {
    with_field(params, "cloudflared_image", json!(image))
  }
}
