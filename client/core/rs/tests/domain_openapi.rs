#![cfg(feature = "utoipa")]
use komodo_client::api::{
  execute::openapi::KomodoExecuteApi, read::openapi::KomodoReadApi,
  write::openapi::KomodoWriteApi,
};
use utoipa::OpenApi;

#[test]
fn complete_openapi_documents_expose_domain_requests_and_responses() {
  let read = serde_json::to_value(KomodoReadApi::openapi()).unwrap();
  let write =
    serde_json::to_value(KomodoWriteApi::openapi()).unwrap();
  let execute =
    serde_json::to_value(KomodoExecuteApi::openapi()).unwrap();
  for (document, methods) in [
    (
      &read,
      vec![
        "GetDomain",
        "ListDomains",
        "GetDomainStatus",
        "GetDnsProvider",
        "GetIngress",
        "PlanDomainChange",
        "GetDomainPlan",
        "ListDomainAdapters",
        "GetDomainAdapterCapabilities",
        "ValidateDomainConnection",
      ],
    ),
    (
      &write,
      vec![
        "CreateDomain",
        "UpdateDomain",
        "DeleteDomain",
        "CreateDnsProvider",
        "UpdateDnsProvider",
        "DeleteDnsProvider",
        "CreateIngress",
        "UpdateIngress",
        "DeleteIngress",
      ],
    ),
    (
      &execute,
      vec![
        "ReconcileDomain",
        "RemoveDomain",
        "MigrateDomain",
        "FinalizeDomainMigration",
      ],
    ),
  ] {
    // Serialize the complete document (not a hand-written partial schema) to
    // catch recursive schema failures and absent domain registrations.
    assert!(!serde_json::to_string(document).unwrap().is_empty());
    for method in methods {
      let operation =
        &document["paths"][format!("/{method}")]["post"];
      assert!(operation["requestBody"]["content"]["application/json"]["schema"].is_object(),"missing request schema: {method}");
      assert!(
        operation["responses"]["200"]["content"]["application/json"]
          ["schema"]
          .is_object(),
        "missing response schema: {method}"
      );
    }
  }
  // Aliases of Resource must not collapse into a shared "Resource" schema:
  // otherwise the last connection response overwrites Domain.config/info.
  for (method, config, info) in [
    ("GetDomain", "DomainConfig", "DomainInfo"),
    ("GetDnsProvider", "DnsProviderConfig", "ConnectionInfo"),
    ("GetIngress", "IngressConfig", "ConnectionInfo"),
  ] {
    let response = &read["paths"][format!("/{method}")]["post"]["responses"]
      ["200"]["content"]["application/json"]["schema"];
    let reference =
      response["$ref"].as_str().expect("resource reference");
    let schema = &read["components"]["schemas"]
      [reference.rsplit('/').next().unwrap()];
    assert_eq!(
      resolve_schema(&read, &schema["properties"]["config"]),
      &read["components"]["schemas"][config]
    );
    assert_eq!(
      resolve_schema(&read, &schema["properties"]["info"]),
      &read["components"]["schemas"][info]
    );
  }
  for (list, get) in [
    ("ListDomains", "GetDomain"),
    ("ListDnsProviders", "GetDnsProvider"),
    ("ListIngresses", "GetIngress"),
  ] {
    let listed = &read["paths"][format!("/{list}")]["post"]["responses"]
      ["200"]["content"]["application/json"]["schema"];
    let single = &read["paths"][format!("/{get}")]["post"]["responses"]
      ["200"]["content"]["application/json"]["schema"];
    assert_eq!(listed["type"], "array");
    assert_eq!(listed["items"]["$ref"], single["$ref"]);
  }
  assert!(read["components"]["schemas"]["DomainInfo"]["properties"]["generation"].is_object());
  for schema in [
    "DomainConfig",
    "DomainInfo",
    "DomainChangeIntent",
    "DnsProviderConfig",
    "IngressConfig",
  ] {
    assert!(
      read["components"]["schemas"][schema].is_object(),
      "missing shared schema: {schema}"
    );
  }
}

// Utoipa may inline concrete generic arguments or reference components. Compare
// their complete schemas so either representation is valid, while overwritten
// resource fields and unconstrained objects still fail the contract.
fn resolve_schema<'a>(
  document: &'a serde_json::Value,
  schema: &'a serde_json::Value,
) -> &'a serde_json::Value {
  match schema["$ref"].as_str() {
    Some(reference) => document
      .pointer(
        reference.strip_prefix('#').expect("local schema reference"),
      )
      .expect("referenced schema exists"),
    None => schema,
  }
}
