import { komodo_client, useRead } from "@/lib/hooks";
import { useQueries } from "@tanstack/react-query";
import { Types } from "komodo_client";

export type ServiceTarget = Extract<
  Types.ResourceTarget,
  { type: "Stack" | "Deployment" }
>;

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
