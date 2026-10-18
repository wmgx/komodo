//! Resolve Compose services using observed network membership and labels.
//! Never route to ephemeral container IPs or infer cross-host reachability.
use anyhow::{Context, Result, bail};
use komodo_client::entities::{
  deployment::Deployment,
  docker::container::{Container, ContainerStateStatusEnum},
  domain::{Ingress, IngressAdapterConfig, TargetScheme},
  server::Server,
  stack::Stack,
};

pub async fn stack_service(
  stack: &Stack,
  service: &str,
  port: u16,
  scheme: &TargetScheme,
  ingress: &Ingress,
) -> Result<String> {
  let server_id = match &ingress.config.adapter {
    IngressAdapterConfig::TraefikFileV1 { server, .. } => server,
    _ => bail!(
      "UnsupportedCapability: Tunnel requires an explicitly allowed URL target"
    ),
  };
  let network = &ingress.config.docker_network;
  if !stack.config.swarm_id.is_empty()
    || stack.config.server_id != *server_id
    || network.is_empty()
    || matches!(network.as_str(), "bridge" | "host" | "none")
  {
    bail!(
      "UnsupportedCapability: Compose target requires the ingress server and a custom Docker network"
    );
  }
  let containers = observe_network(server_id, network).await?;
  verified_service_backend(
    &containers,
    &stack.project_name(false),
    service,
    network,
    port,
    scheme,
  )
}

async fn observe_network(
  server_id: &str,
  network: &str,
) -> Result<Vec<Container>> {
  let server = crate::resource::get::<Server>(server_id).await?;
  let client = crate::helpers::periphery_client(&server).await?;
  let observed = client
    .request(periphery_client::api::docker::InspectNetwork {
      name: network.to_owned(),
    })
    .await
    .map_err(|_| {
      anyhow::anyhow!(
        "TargetUnresolved: ingress network inspection failed"
      )
    })?;
  if observed.scope.as_deref() != Some("local")
    || observed.containers.len() > 256
  {
    bail!(
      "UnsupportedCapability: only bounded local Docker networks are supported"
    );
  }
  let mut containers = Vec::with_capacity(observed.containers.len());
  for member in observed.containers {
    if member.container_id.is_empty() {
      bail!(
        "TargetUnresolved: network member has no stable container ID"
      )
    }
    containers.push(client.request(periphery_client::api::container::InspectContainer {name:member.container_id}).await.map_err(|_|anyhow::anyhow!("TargetUnresolved: network membership changed during inspection"))?);
  }
  Ok(containers)
}

pub async fn deployment(
  deployment: &Deployment,
  port: u16,
  scheme: &TargetScheme,
  ingress: &Ingress,
) -> Result<String> {
  let server = match &ingress.config.adapter {
    IngressAdapterConfig::TraefikFileV1 { server, .. } => server,
    _ => bail!(
      "UnsupportedCapability: Tunnel requires an explicitly allowed URL target"
    ),
  };
  let network = &ingress.config.docker_network;
  if !deployment.config.swarm_id.is_empty()
    || deployment.config.server_id != *server
    || deployment.config.network != *network
    || network.is_empty()
    || matches!(network.as_str(), "bridge" | "host" | "none")
  {
    bail!(
      "UnsupportedCapability: Deployment must share the ingress server and custom Docker network"
    );
  }
  let containers = observe_network(server, network).await?;
  verified_deployment_backend(
    &containers,
    deployment.deployed_name(),
    network,
    port,
    scheme,
  )
}

fn verified_deployment_backend(
  containers: &[Container],
  name: &str,
  network: &str,
  port: u16,
  scheme: &TargetScheme,
) -> Result<String> {
  if port == 0
    || name.is_empty()
    || !name.chars().all(|c| {
      c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
    })
  {
    bail!("InvalidConfig: invalid deployment name or port")
  }
  let mut matching = 0;
  for container in containers {
    let endpoint = container
      .network_settings
      .as_ref()
      .and_then(|settings| settings.networks.get(network))
      .context(
        "TargetUnresolved: deployment network membership changed",
      )?;
    let exact_name = container
      .name
      .as_deref()
      .is_some_and(|value| value.trim_start_matches('/') == name);
    let exposes_alias = endpoint
      .aliases
      .iter()
      .chain(endpoint.dns_names.iter())
      .any(|alias| alias.eq_ignore_ascii_case(name))
      || container.name.as_deref().is_some_and(|value| {
        value.trim_start_matches('/').eq_ignore_ascii_case(name)
      });
    if exposes_alias && !exact_name {
      bail!(
        "Conflict: deployment DNS name is shared with another container"
      )
    }
    if exact_name {
      if !container.state.as_ref().is_some_and(|state| {
        state.status == ContainerStateStatusEnum::Running
      }) {
        bail!("TargetUnresolved: Deployment container is not running")
      }
      matching += 1;
    }
  }
  if matching != 1 {
    bail!(
      "TargetUnresolved: expected running Deployment container is absent"
    )
  }
  let scheme = match scheme {
    TargetScheme::Http => "http",
    TargetScheme::Https => "https",
  };
  Ok(format!("{scheme}://{name}:{port}"))
}

fn verified_service_backend(
  containers: &[Container],
  project: &str,
  service: &str,
  network: &str,
  port: u16,
  scheme: &TargetScheme,
) -> Result<String> {
  if port == 0
    || service.is_empty()
    || !service
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
  {
    bail!("InvalidConfig: invalid stack service or port");
  }
  let mut matching = 0;
  for container in containers {
    let endpoint = container.network_settings.as_ref().and_then(|settings|settings.networks.get(network)).context("TargetUnresolved: network membership changed during inspection")?;
    let exposes_alias = endpoint
      .aliases
      .iter()
      .chain(endpoint.dns_names.iter())
      .any(|name| name == service);
    let owned_service =
      container.config.as_ref().is_some_and(|config| {
        config
          .labels
          .get("com.docker.compose.project")
          .map(String::as_str)
          == Some(project)
          && config
            .labels
            .get("com.docker.compose.service")
            .map(String::as_str)
            == Some(service)
      });
    if exposes_alias && !owned_service {
      bail!(
        "Conflict: service DNS alias is shared with another target"
      )
    }
    if owned_service {
      if !exposes_alias {
        bail!(
          "TargetUnresolved: Compose service has no stable alias on ingress network"
        )
      }
      if !container.state.as_ref().is_some_and(|state| {
        state.status == ContainerStateStatusEnum::Running
      }) {
        bail!("TargetUnresolved: Compose service is not running")
      }
      matching += 1;
    }
  }
  if matching != 1 {
    bail!(
      "UnsupportedCapability: exactly one running Compose service endpoint is required"
    )
  }
  let scheme = match scheme {
    TargetScheme::Http => "http",
    TargetScheme::Https => "https",
  };
  Ok(format!("{scheme}://{service}:{port}"))
}

#[cfg(test)]
mod tests {
  use super::*;
  fn container(
    project: &str,
    service: &str,
    alias: &str,
  ) -> Container {
    serde_json::from_value(serde_json::json!({
      "Config":{"Labels":{"com.docker.compose.project":project,"com.docker.compose.service":service}},
      "State":{"Status":"running"},
      "NetworkSettings":{"Networks":{"edge":{"Aliases":[alias]}}}
    })).unwrap()
  }
  #[test]
  fn deployment_uses_observed_deployed_name_and_rejects_alias_collision()
   {
    let mut good = container("unused", "unused", "old-custom-name");
    good.name = Some("/old-custom-name".into());
    assert_eq!(
      verified_deployment_backend(
        &[good.clone()],
        "old-custom-name",
        "edge",
        8080,
        &TargetScheme::Https
      )
      .unwrap(),
      "https://old-custom-name:8080"
    );
    assert!(
      verified_deployment_backend(
        &[good.clone()],
        "new-configured-name",
        "edge",
        8080,
        &TargetScheme::Http
      )
      .is_err()
    );
    let mut impostor = good.clone();
    impostor.name = Some("/other".into());
    assert!(
      verified_deployment_backend(
        &[good, impostor],
        "old-custom-name",
        "edge",
        8080,
        &TargetScheme::Http
      )
      .is_err()
    );
  }
  #[test]
  fn stable_service_name_requires_observed_labels_and_network_alias()
  {
    let good = container("app", "web", "web");
    assert_eq!(
      verified_service_backend(
        &[good.clone()],
        "app",
        "web",
        "edge",
        8080,
        &TargetScheme::Http
      )
      .unwrap(),
      "http://web:8080"
    );
    assert!(
      verified_service_backend(
        &[good.clone(), container("other", "web", "web")],
        "app",
        "web",
        "edge",
        8080,
        &TargetScheme::Http
      )
      .is_err()
    );
    assert!(
      verified_service_backend(
        &[container("app", "web", "other")],
        "app",
        "web",
        "edge",
        8080,
        &TargetScheme::Http
      )
      .is_err()
    );
    assert!(
      verified_service_backend(
        &[good.clone(), good],
        "app",
        "web",
        "edge",
        8080,
        &TargetScheme::Http
      )
      .is_err()
    );
  }
}
