//! Domain management: DNS records + tunnel ingress rules declared by
//! `Domain` resources and reconciled on demand (plan / apply).
//!
//! Hard rules for this module: no `unwrap` / `expect` / `panic!` /
//! `process::exit` in non-test code; every failure is a [DomainError]
//! with a stable code.
#![cfg_attr(
  not(test),
  deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod accounts;
pub mod apply;
pub mod auto;
pub mod error;
pub mod lock;
pub mod plan;
pub mod prepare;
pub mod provider;
pub mod service;
pub mod store;
pub mod validate;
