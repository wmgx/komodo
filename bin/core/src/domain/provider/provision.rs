//! Optional capability: open an ingress (tunnel) by itself.
//!
//! The provider creates the remote object and describes how to run its
//! connector; Core orchestrates the Deployment. Nothing here is vendor
//! specific.

use serde_json::Value;

use super::{BoxFut, ServerRef};

/// A tunnel bound to a Server.
#[derive(Debug, Clone)]
pub struct TunnelHandle {
  /// Registered name (what Domains may reference).
  pub name: String,
  /// Provider container (eg. Cloudflare account id).
  pub scope: String,
  /// Provider object id.
  pub id: String,
  /// Created by Komodo, so Komodo may delete it.
  pub managed: bool,
}

/// How to run the connector: always host network, restart unless stopped.
/// Holds a secret in `env`, never log it.
#[derive(Clone)]
pub struct ConnectorSpec {
  pub image: String,
  pub command: String,
  pub env: Vec<(String, String)>,
}

pub struct Provisioned {
  pub handle: TunnelHandle,
  pub connector: ConnectorSpec,
}

pub trait ProvisionProvider: Send + Sync {
  /// Connector image when the caller gives none: the account's
  /// remembered image, else the vendor's official one.
  fn default_image(&self) -> String;

  /// The tunnel already bound to `server` in the account params.
  fn registered(&self, server: &ServerRef) -> Option<TunnelHandle>;

  /// Create the tunnel with an empty (catch-all 404) configuration and
  /// fetch the connector credentials. Removes the tunnel again if a later
  /// step fails, so a failure leaves nothing behind.
  fn provision<'a>(
    &'a self,
    name: &'a str,
    image: &'a str,
  ) -> BoxFut<'a, Provisioned>;

  /// Connector description for an existing tunnel (retry path).
  fn connector<'a>(
    &'a self,
    handle: &'a TunnelHandle,
    image: &'a str,
  ) -> BoxFut<'a, ConnectorSpec>;

  /// Provider side connection status, eg. `healthy`.
  fn status<'a>(
    &'a self,
    handle: &'a TunnelHandle,
  ) -> BoxFut<'a, String>;

  /// Delete the tunnel at the provider. No-op if already gone.
  fn destroy<'a>(
    &'a self,
    handle: &'a TunnelHandle,
  ) -> BoxFut<'a, ()>;

  /// New account params with the tunnel bound to `server` and `image`
  /// remembered. Unknown fields are kept.
  fn register(
    &self,
    params: &Value,
    handle: &TunnelHandle,
    server: &ServerRef,
    image: &str,
  ) -> Value;

  /// New account params without the tunnel.
  fn unregister(
    &self,
    params: &Value,
    handle: &TunnelHandle,
  ) -> Value;

  /// New account params remembering `image`.
  fn remember_image(&self, params: &Value, image: &str) -> Value;
}
