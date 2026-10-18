use std::fmt;

use komodo_client::entities::domain::{DomainLayer, DomainReason};

/// Stable error codes. AI / UI can switch on these.
pub mod code {
  pub const ACCOUNT_NOT_FOUND: &str = "AccountNotFound";
  pub const ACCOUNT_KIND_MISMATCH: &str = "AccountKindMismatch";
  pub const INVALID_CONFIG: &str = "InvalidConfig";
  pub const ZONE_NOT_FOUND: &str = "ZoneNotFound";
  pub const TUNNEL_NOT_FOUND: &str = "TunnelNotFound";
  pub const TUNNEL_AMBIGUOUS: &str = "TunnelAmbiguous";
  pub const AUTH_FAILED: &str = "AuthFailed";
  pub const RATE_LIMITED: &str = "RateLimited";
  pub const PROVIDER_UNAVAILABLE: &str = "ProviderUnavailable";
  pub const PROVIDER_REJECTED: &str = "ProviderRejected";
  pub const PROVIDER_RESPONSE: &str = "ProviderResponse";
  pub const TIMEOUT: &str = "Timeout";
  pub const FOREIGN_CONFLICT: &str = "ForeignRecordConflict";
  pub const PLAN_CHANGED: &str = "PlanChanged";
  pub const INTERNAL: &str = "Internal";
}

/// Error returned by every domain operation.
///
/// `message` is safe to show to users (no tokens, no provider response
/// bodies). `detail` keeps the original cause for server side logs only.
#[derive(Debug, Clone)]
pub struct DomainError {
  pub code: &'static str,
  pub message: String,
  pub retryable: bool,
  pub detail: Option<String>,
}

/// Errors from provider implementations share the same type.
pub type ProviderError = DomainError;

impl DomainError {
  pub fn new(code: &'static str, message: impl Into<String>) -> Self {
    Self {
      code,
      message: message.into(),
      retryable: false,
      detail: None,
    }
  }

  pub fn invalid(message: impl Into<String>) -> Self {
    Self::new(code::INVALID_CONFIG, message)
  }

  pub fn retryable(mut self) -> Self {
    self.retryable = true;
    self
  }

  /// Attach the original cause. Must not contain tokens.
  pub fn with_detail(mut self, detail: impl fmt::Display) -> Self {
    self.detail = Some(detail.to_string());
    self
  }

  pub fn internal(cause: impl fmt::Display) -> Self {
    Self::new(code::INTERNAL, "Internal error, see Core logs")
      .with_detail(cause)
  }

  /// Log the original cause on the server.
  pub fn log(&self, context: &str) {
    error!(
      code = self.code,
      retryable = self.retryable,
      "Domain error | {context} | {} | {}",
      self.message,
      self.detail.as_deref().unwrap_or("-")
    );
  }

  pub fn to_reason(
    &self,
    layer: Option<DomainLayer>,
    object_key: Option<&str>,
  ) -> DomainReason {
    DomainReason {
      code: self.code.to_string(),
      layer,
      object_key: object_key.map(str::to_string),
      message: self.message.clone(),
      retryable: self.retryable,
    }
  }
}

impl fmt::Display for DomainError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}: {}", self.code, self.message)
  }
}

impl std::error::Error for DomainError {}

/// Replace every occurrence of `secret` in `text`.
pub fn redact(text: &str, secret: &str) -> String {
  if secret.is_empty() {
    text.to_string()
  } else {
    text.replace(secret, "***")
  }
}

impl DomainError {
  /// HTTP status to use when this error is returned from the API.
  pub fn status(&self) -> reqwest::StatusCode {
    use reqwest::StatusCode;
    match self.code {
      code::ACCOUNT_NOT_FOUND
      | code::ACCOUNT_KIND_MISMATCH
      | code::INVALID_CONFIG
      | code::ZONE_NOT_FOUND
      | code::TUNNEL_NOT_FOUND
      | code::TUNNEL_AMBIGUOUS => StatusCode::BAD_REQUEST,
      code::FOREIGN_CONFLICT | code::PLAN_CHANGED => {
        StatusCode::CONFLICT
      }
      code::RATE_LIMITED => StatusCode::TOO_MANY_REQUESTS,
      code::TIMEOUT => StatusCode::GATEWAY_TIMEOUT,
      code::AUTH_FAILED
      | code::PROVIDER_UNAVAILABLE
      | code::PROVIDER_REJECTED
      | code::PROVIDER_RESPONSE => StatusCode::BAD_GATEWAY,
      _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
  }

  /// Convert for API responses. Only code + safe message are exposed.
  pub fn into_api(self) -> mogh_error::Error {
    use mogh_error::AddStatusCodeError;
    let status = self.status();
    self.status_code(status)
  }
}
