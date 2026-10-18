import { useListItem, useRead } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import { RequiredResourceComponents } from "..";
import { Types } from "komodo_client";
import NewResource from "@/resources/new";
import ResourceHeader from "../header";
import DomainTable from "./table";
import DomainConfig from "./config";
import DomainPlan from "./plan";
import { domainStatusIntention } from "@/lib/color";
import { hexColorByIntention, StatusBadge } from "mogh_ui";
import { Group, Text } from "@mantine/core";

export function useDomain(
  id: string | undefined,
  useName?: boolean,
  refetchInterval?: number | false,
) {
  return useListItem("Domain", id, useName, refetchInterval);
}

export function useFullDomain(id: string) {
  return useRead("GetDomain", { domain: id }, { refetchInterval: 30_000 }).data;
}

export const DomainComponents: RequiredResourceComponents<
  Types.DomainConfig,
  Types.DomainInfo,
  Types.DomainListItemInfo,
  Types.DomainQuerySpecifics
> = {
  useList: (query, limit, page) =>
    useRead("ListDomains", { query, limit, page }).data,
  useListItem: useDomain,
  useFull: useFullDomain,

  useResourceLinks: (domain) =>
    domain?.config?.hostname ? [`https://${domain.config.hostname}`] : undefined,

  useDashboardSummaryData: () => {
    const domains = useRead("ListDomains", {}, { refetchInterval: 10_000 }).data;
    const count = (code: string) =>
      domains?.filter((d) => d.info.status_code === code).length ?? 0;
    return [
      { title: "Ready", intention: "Good", value: count("ready") },
      { title: "Drifted", intention: "Warning", value: count("drifted") },
      { title: "Error", intention: "Critical", value: count("error") },
      { title: "Unchecked", intention: "Unknown", value: count("unchecked") },
    ];
  },

  Description: () => <>Manage DNS records and tunnel routes for hostnames.</>,

  New: () => <NewResource type="Domain" />,

  // No batch executions: Apply needs a reviewed plan per Domain.
  BatchExecutions: () => null,

  Table: DomainTable,

  Icon: ({ id, size = "1rem", noColor }) => {
    const code = useDomain(id)?.info.status_code;
    const color =
      !code || noColor
        ? undefined
        : hexColorByIntention(domainStatusIntention(code));
    return <ICONS.Domain size={size} color={color} />;
  },

  ResourcePageHeader: ({ id }) => {
    const domain = useDomain(id);
    return (
      <ResourceHeader
        type="Domain"
        id={id}
        resource={domain}
        intent={domainStatusIntention(domain?.info.status_code)}
        icon={ICONS.Domain}
        name={domain?.name}
        state={domain?.info.status_code}
        status={domain?.info.hostname}
      />
    );
  },

  State: ({ id }) => {
    const code = useDomain(id)?.info.status_code;
    return <StatusBadge text={code} intent={domainStatusIntention(code)} />;
  },

  Info: {
    Reason: ({ id }) => {
      const status = useFullDomain(id)?.info?.status;
      const reasons =
        status?.type === "Error"
          ? [status.data.reason]
          : status?.type === "Drifted"
            ? status.data.reasons
            : [];
      if (!reasons.length) return null;
      return (
        <Group gap="xs">
          {reasons.map((r, i) => (
            <Text key={i} fz="sm" c="dimmed" title={r.code}>
              {r.code}: {r.message}
            </Text>
          ))}
        </Group>
      );
    },
  },

  Executions: {},

  Config: DomainConfig,

  Page: { Plan: DomainPlan },
};
