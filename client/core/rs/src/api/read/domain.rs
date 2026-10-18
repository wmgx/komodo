use super::KomodoReadRequest;
#[cfg(feature = "utoipa")]
use crate::entities::resource::Resource;
use crate::entities::{U64, domain::*};
use mogh_resolver::Resolve;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/GetDomain",request_body(content=GetDomain),responses((status=200,body=Resource<DomainConfig, DomainInfo>)))]
pub fn get_domain() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(Domain)]
#[error(mogh_error::Error)]
pub struct GetDomain {
  pub domain: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/ListDomains",request_body(content=ListDomains),responses((status=200,body=[Resource<DomainConfig, DomainInfo>])))]
pub fn list_domains() {}
#[typeshare]
#[derive(Default, Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(Vec<Domain>)]
#[error(mogh_error::Error)]
pub struct ListDomains {
  #[serde(default)]
  pub query: DomainQuery,
  #[serde(default)]
  pub page: U64,
  pub limit: Option<U64>,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/GetDnsProvider",request_body(content=GetDnsProvider),responses((status=200,body=Resource<DnsProviderConfig, ConnectionInfo>)))]
pub fn get_dns_provider() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(DnsProvider)]
#[error(mogh_error::Error)]
pub struct GetDnsProvider {
  pub dns_provider: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/ListDnsProviders",request_body(content=ListDnsProviders),responses((status=200,body=[Resource<DnsProviderConfig, ConnectionInfo>])))]
pub fn list_dns_providers() {}
#[typeshare]
#[derive(Default, Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(Vec<DnsProvider>)]
#[error(mogh_error::Error)]
pub struct ListDnsProviders {
  #[serde(default)]
  pub query: DnsProviderQuery,
  #[serde(default)]
  pub page: U64,
  pub limit: Option<U64>,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/GetIngress",request_body(content=GetIngress),responses((status=200,body=Resource<IngressConfig, ConnectionInfo>)))]
pub fn get_ingress() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(Ingress)]
#[error(mogh_error::Error)]
pub struct GetIngress {
  pub ingress: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/ListIngresses",request_body(content=ListIngresses),responses((status=200,body=[Resource<IngressConfig, ConnectionInfo>])))]
pub fn list_ingresses() {}
#[typeshare]
#[derive(Default, Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(Vec<Ingress>)]
#[error(mogh_error::Error)]
pub struct ListIngresses {
  #[serde(default)]
  pub query: IngressQuery,
  #[serde(default)]
  pub page: U64,
  pub limit: Option<U64>,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/GetDomainStatus",request_body(content=GetDomainStatus),responses((status=200,body=DomainStatus)))]
pub fn get_domain_status() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(DomainStatus)]
#[error(mogh_error::Error)]
pub struct GetDomainStatus {
  pub domain: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/ListDomainAdapters",request_body(content=ListDomainAdapters),responses((status=200,body=Vec<DomainAdapterCapabilities>)))]
pub fn list_domain_adapters() {}
#[typeshare]
#[derive(Default, Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(Vec<DomainAdapterCapabilities>)]
#[error(mogh_error::Error)]
pub struct ListDomainAdapters {}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/GetDomainAdapterCapabilities",request_body(content=GetDomainAdapterCapabilities),responses((status=200,body=DomainAdapterCapabilities)))]
pub fn get_domain_adapter_capabilities() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(DomainAdapterCapabilities)]
#[error(mogh_error::Error)]
pub struct GetDomainAdapterCapabilities {
  pub adapter: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/PlanDomainChange",request_body(content=PlanDomainChange),responses((status=200,body=DomainPlan)))]
pub fn plan_domain_change() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(DomainPlan)]
#[error(mogh_error::Error)]
pub struct PlanDomainChange {
  pub intent: DomainChangeIntent,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/GetDomainPlan",request_body(content=GetDomainPlan),responses((status=200,body=DomainPlan)))]
pub fn get_domain_plan() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(DomainPlan)]
#[error(mogh_error::Error)]
pub struct GetDomainPlan {
  pub plan_id: String,
}

#[cfg(feature = "utoipa")]
#[utoipa::path(post,path="/ValidateDomainConnection",request_body(content=ValidateDomainConnection),responses((status=200,body=DomainConnectionValidation)))]
pub fn validate_domain_connection() {}
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Resolve)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[empty_traits(KomodoReadRequest)]
#[response(DomainConnectionValidation)]
#[error(mogh_error::Error)]
pub struct ValidateDomainConnection {
  pub connection: DomainConnection,
}
