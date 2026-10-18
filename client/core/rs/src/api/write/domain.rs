use super::KomodoWriteRequest;
#[cfg(feature = "utoipa")]
use crate::entities::resource::Resource;
use crate::entities::{I64, domain::*};
use mogh_resolver::Resolve;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/CreateDomain",request_body(content=CreateDomain),responses((status=200,body=Resource<DomainConfig, DomainInfo>)))]
pub fn create_domain() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(Domain)]
#[error(mogh_error::Error)]
pub struct CreateDomain {
  pub name: String,
  pub config: DomainConfig,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/UpdateDomain",request_body(content=UpdateDomain),responses((status=200,body=Resource<DomainConfig, DomainInfo>)))]
pub fn update_domain() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(Domain)]
#[error(mogh_error::Error)]
pub struct UpdateDomain {
  pub id: String,
  pub config: DomainConfig,
  pub expected_generation: I64,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/DeleteDomain",request_body(content=DeleteDomain),responses((status=200,body=Resource<DomainConfig, DomainInfo>)))]
pub fn delete_domain() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(Domain)]
#[error(mogh_error::Error)]
pub struct DeleteDomain {
  pub id: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/CreateDnsProvider",request_body(content=CreateDnsProvider),responses((status=200,body=Resource<DnsProviderConfig, ConnectionInfo>)))]
pub fn create_dns_provider() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(DnsProvider)]
#[error(mogh_error::Error)]
pub struct CreateDnsProvider {
  pub name: String,
  pub config: DnsProviderConfig,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/UpdateDnsProvider",request_body(content=UpdateDnsProvider),responses((status=200,body=Resource<DnsProviderConfig, ConnectionInfo>)))]
pub fn update_dns_provider() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(DnsProvider)]
#[error(mogh_error::Error)]
pub struct UpdateDnsProvider {
  pub id: String,
  pub config: DnsProviderConfig,
  pub expected_generation: I64,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/DeleteDnsProvider",request_body(content=DeleteDnsProvider),responses((status=200,body=Resource<DnsProviderConfig, ConnectionInfo>)))]
pub fn delete_dns_provider() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(DnsProvider)]
#[error(mogh_error::Error)]
pub struct DeleteDnsProvider {
  pub id: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/CreateIngress",request_body(content=CreateIngress),responses((status=200,body=Resource<IngressConfig, ConnectionInfo>)))]
pub fn create_ingress() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(Ingress)]
#[error(mogh_error::Error)]
pub struct CreateIngress {
  pub name: String,
  pub config: IngressConfig,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/UpdateIngress",request_body(content=UpdateIngress),responses((status=200,body=Resource<IngressConfig, ConnectionInfo>)))]
pub fn update_ingress() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(Ingress)]
#[error(mogh_error::Error)]
pub struct UpdateIngress {
  pub id: String,
  pub config: IngressConfig,
  pub expected_generation: I64,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/DeleteIngress",request_body(content=DeleteIngress),responses((status=200,body=Resource<IngressConfig, ConnectionInfo>)))]
pub fn delete_ingress() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoWriteRequest)]
#[response(Ingress)]
#[error(mogh_error::Error)]
pub struct DeleteIngress {
  pub id: String,
}
