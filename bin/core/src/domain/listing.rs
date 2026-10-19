//! "Which services run where, on which ports, and are they public?"
//! Answered from the status caches and the Domain list only: no call to
//! Periphery or to a provider, so it is cheap to poll.

use std::net::IpAddr;

use anyhow::bail;
use komodo_client::entities::{
  ResourceTarget,
  deployment::Deployment,
  docker::container::{Port, PortTypeEnum},
  domain::{Domain, ServiceItem, ServicePort, ServicePublicUrl},
  permission::PermissionLevel,
  stack::Stack,
  user::User,
};

use crate::{
  resource,
  state::{
    all_resources_cache, deployment_status_cache, stack_status_cache,
  },
};

use super::{
  expose::stage_of, prepare::find_by_id_or_name, service::server_info,
};

/// `http://<host>:<port>`, unless the host is missing or only the Core
/// machine itself.
fn lan_url(host: Option<&str>, port: u16) -> Option<String> {
  let host = host?;
  match host.parse::<IpAddr>() {
    Ok(ip) if ip.is_loopback() || ip.is_unspecified() => None,
    Ok(IpAddr::V6(ip)) => Some(format!("http://[{ip}]:{port}")),
    _ if host.eq_ignore_ascii_case("localhost") => None,
    _ => Some(format!("http://{host}:{port}")),
  }
}

/// Published TCP ports, one entry per host port, sorted.
pub fn ports_of(
  ports: &[Port],
  host: Option<&str>,
) -> Vec<ServicePort> {
  let mut out: Vec<ServicePort> = ports
    .iter()
    .filter(|p| {
      matches!(p.typ, PortTypeEnum::TCP | PortTypeEnum::EMPTY)
    })
    .filter_map(|p| {
      let host_port = p.public_port.filter(|port| *port != 0)?;
      Some(ServicePort {
        host: host_port,
        container: p.private_port,
        lan_url: lan_url(host, host_port),
      })
    })
    .collect();
  out.sort_by_key(|p| (p.host, p.container));
  out.dedup_by_key(|p| p.host);
  out
}

/// Port which most likely speaks HTTP: a well known web port, else the
/// first one.
pub fn choose_port(ports: &[ServicePort]) -> Option<u16> {
  const WEB: [u16; 8] = [80, 8080, 3000, 9120, 8000, 8888, 443, 5000];
  WEB
    .iter()
    .find(|w| ports.iter().any(|p| p.host == **w))
    .copied()
    .or_else(|| ports.first().map(|p| p.host))
}

/// Published ports of a Deployment / Stack service, from the caches.
pub async fn service_ports(
  target: &ResourceTarget,
  service: &str,
  server_id: &str,
) -> Vec<ServicePort> {
  let host = server_info(server_id).and_then(|s| s.host);
  let ports = match target {
    ResourceTarget::Deployment(id) => deployment_status_cache()
      .get(id)
      .await
      .and_then(|s| s.curr.container.clone())
      .map(|c| c.ports)
      .unwrap_or_default(),
    ResourceTarget::Stack(id) => stack_status_cache()
      .get(id)
      .await
      .and_then(|s| {
        s.curr
          .services
          .iter()
          .find(|s| s.service == service)
          .and_then(|s| s.container.clone())
      })
      .map(|c| c.ports)
      .unwrap_or_default(),
    _ => Vec::new(),
  };
  ports_of(&ports, host.as_deref())
}

struct Bound {
  target: Option<ResourceTarget>,
  service: String,
  public: ServicePublicUrl,
}

pub async fn list_services(
  user: &User,
  server: Option<&str>,
) -> anyhow::Result<Vec<ServiceItem>> {
  let only_server = match server.filter(|s| !s.is_empty()) {
    Some(s) => {
      let all = all_resources_cache().load();
      match find_by_id_or_name(&all.servers, s, |sv| &sv.name) {
        Some(found) => Some(found.id.clone()),
        None => bail!("Server '{s}' does not exist"),
      }
    }
    None => None,
  };
  let on_server =
    |id: &str| only_server.as_deref().is_none_or(|only| only == id);

  let read = PermissionLevel::Read.into();
  let stacks = resource::list_for_user::<Stack>(
    Default::default(),
    None,
    None,
    user,
    read,
    &[],
  )
  .await?;
  let deployments = resource::list_for_user::<Deployment>(
    Default::default(),
    None,
    None,
    user,
    PermissionLevel::Read.into(),
    &[],
  )
  .await?;
  let domains = resource::list_items_for_user::<Domain>(
    Default::default(),
    resource::ListItemsQueryOptions {
      limit: 0,
      page: 0,
      sort_desc: false,
      sort_by: resource::ListItemSort::Name,
    },
    user,
    PermissionLevel::Read.into(),
    &[],
    |_| true,
  )
  .await?;
  let bound: Vec<Bound> = domains
    .into_iter()
    .map(|d| Bound {
      public: ServicePublicUrl {
        url: if d.info.ingress_account.is_empty() {
          format!("http://{}:{}", d.info.hostname, d.info.port)
        } else {
          format!("https://{}", d.info.hostname)
        },
        domain: d.name,
        domain_id: d.id,
        port: d.info.port,
        status: d.info.status_code,
      },
      target: d.info.target,
      service: d.info.service,
    })
    .collect();
  let public_of = |target: &ResourceTarget, service: &str| {
    bound
      .iter()
      .filter(|b| {
        b.target.as_ref() == Some(target) && b.service == service
      })
      .map(|b| b.public.clone())
      .collect::<Vec<_>>()
  };

  let mut out = Vec::new();
  for d in deployments {
    if !on_server(&d.info.server_id) {
      continue;
    }
    let target = ResourceTarget::Deployment(d.id.clone());
    out.push(ServiceItem {
      ports: service_ports(&target, "", &d.info.server_id).await,
      public: public_of(&target, ""),
      expose: stage_of(&target, ""),
      target,
      service: String::new(),
      name: d.name,
      stack: String::new(),
      server_id: d.info.server_id,
      server_name: d.info.server_name,
      state: format!("{:?}", d.info.state),
    });
  }
  for s in stacks {
    if !on_server(&s.info.server_id) {
      continue;
    }
    let target = ResourceTarget::Stack(s.id.clone());
    // Live services when known, else the declared ones.
    let live = stack_status_cache()
      .get(&s.id)
      .await
      .map(|c| c.curr.services.clone())
      .unwrap_or_default();
    let services: Vec<(String, String)> = if live.is_empty() {
      s.info
        .services
        .iter()
        .map(|x| (x.service.clone(), "Unknown".to_string()))
        .collect()
    } else {
      live
        .iter()
        .map(|x| (x.service.clone(), format!("{:?}", x.state)))
        .collect()
    };
    for (service, state) in services {
      out.push(ServiceItem {
        ports: service_ports(&target, &service, &s.info.server_id)
          .await,
        public: public_of(&target, &service),
        expose: stage_of(&target, &service),
        target: target.clone(),
        name: service.clone(),
        service,
        stack: s.name.clone(),
        server_id: s.info.server_id.clone(),
        server_name: s.info.server_name.clone(),
        state,
      });
    }
  }
  out.sort_by(|a, b| {
    (&a.server_name, &a.stack, &a.name).cmp(&(
      &b.server_name,
      &b.stack,
      &b.name,
    ))
  });
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn port(
    private: u16,
    public: Option<u16>,
    typ: PortTypeEnum,
  ) -> Port {
    Port {
      ip: None,
      private_port: private,
      public_port: public,
      typ,
    }
  }

  #[test]
  fn published_ports_become_lan_urls_and_pick_the_web_port() {
    let raw = [
      port(22, Some(2222), PortTypeEnum::TCP),
      port(80, Some(8080), PortTypeEnum::TCP),
      // The same mapping again (IPv6 listing), unpublished and UDP ones.
      port(80, Some(8080), PortTypeEnum::TCP),
      port(9000, None, PortTypeEnum::TCP),
      port(53, Some(53), PortTypeEnum::UDP),
    ];
    let ports = ports_of(&raw, Some("10.0.0.5"));
    assert_eq!(
      ports
        .iter()
        .map(|p| (p.host, p.container))
        .collect::<Vec<_>>(),
      [(2222, 22), (8080, 80)]
    );
    assert_eq!(
      ports[1].lan_url.as_deref(),
      Some("http://10.0.0.5:8080")
    );
    assert_eq!(choose_port(&ports), Some(8080));
    assert!(ports_of(&raw, Some("127.0.0.1"))[0].lan_url.is_none());
  }
}
