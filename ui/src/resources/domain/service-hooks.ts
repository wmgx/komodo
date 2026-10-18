import { komodo_client, useRead } from "@/lib/hooks";
import { useServerAddress } from "@/resources/server/hooks";
import { useListItemQuery } from "@/lib/hooks";
import { useQueries } from "@tanstack/react-query";
import { Types } from "komodo_client";
import { isPrivateHost } from "./text";

export type ServiceTarget = Extract<
  Types.ResourceTarget,
  { type: "Stack" | "Deployment" }
>;

/** Host ports from lines like `8080:80`, `127.0.0.1:8080:80/tcp`. */
export function parseHostPorts(ports: string | undefined): number[] {
  const out = (ports ?? "")
    .split("\n")
    .map((l) => l.split("#")[0].split("/")[0].trim())
    .filter(Boolean)
    .map((l) => {
      const parts = l.split(":");
      return parts.length >= 2 ? Number(parts[parts.length - 2]) : NaN;
    })
    .filter((p) => Number.isInteger(p) && p > 0 && p < 65536);
  return [...new Set(out)];
}

/** Services of a Stack (names) and published ports of the chosen service. */
export function useServicePorts(target: ServiceTarget, service: string) {
  const isStack = target.type === "Stack";
  const services = useRead(
    "ListStackServices",
    { stack: target.id },
    { enabled: isStack },
  ).data;
  const deployment = useRead(
    "GetDeployment",
    { deployment: target.id },
    { enabled: !isStack },
  ).data;
  if (!isStack) {
    return {
      serviceNames: [] as string[],
      ports: parseHostPorts(deployment?.config?.ports),
    };
  }
  const ports = (services ?? [])
    .filter((s) => s.service === service)
    .flatMap((s) => s.container?.ports ?? [])
    .map((p) => p.PublicPort)
    .filter((p): p is number => !!p);
  return {
    serviceNames: (services ?? []).map((s) => s.service),
    ports: [...new Set(ports)],
  };
}

export type ZoneOption = { zone: string; accounts: string[] };

/** Zones of every DNS capable account (read only discovery, admin only). */
export function useZones(enabled: boolean) {
  const accounts = useRead("ListDomainAccounts", {}, { enabled }).data;
  const dnsAccounts = (accounts ?? []).filter((a) =>
    a.capabilities.includes(Types.DomainLayer.Dns),
  );
  const results = useQueries({
    queries: dnsAccounts.map((a) => ({
      queryKey: ["DiscoverDomainAccount", { account: a.name }],
      queryFn: () =>
        komodo_client().read("DiscoverDomainAccount", { account: a.name }),
      enabled,
      staleTime: 60_000,
      retry: false,
    })),
  });
  const map = new Map<string, Set<string>>();
  dnsAccounts.forEach((a, i) => {
    const whitelist: string[] = a.params?.zones ?? [];
    (results[i].data?.zones ?? [])
      .filter((z) => !whitelist.length || whitelist.includes(z))
      .forEach((z) => {
        if (!map.has(z)) map.set(z, new Set());
        map.get(z)!.add(a.name);
      });
  });
  const zones: ZoneOption[] = [...map.entries()]
    .map(([zone, set]) => ({ zone, accounts: [...set] }))
    .sort((a, b) => a.zone.localeCompare(b.zone));
  return {
    zones,
    accounts,
    loading: results.some((r) => r.isLoading) || (enabled && !accounts),
    failed: results.some((r) => r.isError),
  };
}

/**
 * Same rule as the backend (`domain/auto.rs`), only used to explain what
 * will happen before the user clicks Add. The backend decides for real.
 */
export function useAccessHint(
  target: ServiceTarget,
  accounts: Types.DomainAccount[] | undefined,
) {
  const q = useListItemQuery(target.type, target.id);
  const serverId = (
    q.data?.[0] as { info?: { server_id?: string } } | undefined
  )?.info?.server_id;
  const server = useRead(
    "ListServers",
    {},
    { enabled: !!serverId },
  ).data?.find((s) => s.id === serverId);
  const address = useServerAddress(serverId);
  const tunnel = (accounts ?? []).some((a) =>
    ((a.params?.tunnels as { server?: string }[] | undefined) ?? []).some(
      (t) => t.server && (t.server === serverId || t.server === server?.name),
    ),
  );
  const host = address?.hostname;
  return {
    serverName: server?.name,
    tunnel,
    host,
    lanOnly: !tunnel && !!host && isPrivateHost(host),
    unknown: !tunnel && !host,
  };
}
