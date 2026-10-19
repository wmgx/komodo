import { useRead, useSetTitle } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import ResourceLink from "@/resources/link";
import { ServiceAccessButton } from "@/resources/domain/make-public";
import { domainStatusIntention } from "@/lib/color";
import { Anchor, Group, Select, Stack, Text } from "@mantine/core";
import { Types } from "komodo_client";
import { DataTable, Page, SearchInput, SortableHeader, StatusBadge } from "mogh_ui";
import { useMemo, useState } from "react";

type Intent = "Good" | "Warning" | "Critical" | "Unknown" | "None";

function stateIntent(state: string): Intent {
  switch (state) {
    case "Running":
    case "Healthy":
      return "Good";
    case "Exited":
    case "Dead":
    case "Down":
    case "Unhealthy":
      return "Critical";
    case "NotDeployed":
    case "Unknown":
      return "None";
    default:
      return "Warning";
  }
}

const Links = ({ urls }: { urls: string[] }) =>
  urls.length ? (
    <Stack gap={2}>
      {urls.map((u) => (
        <Anchor key={u} href={u} target="_blank" rel="noreferrer" fz="sm">
          {u}
        </Anchor>
      ))}
    </Stack>
  ) : (
    <Text c="dimmed">-</Text>
  );

const matches = (s: Types.ServiceItem, terms: string[]) => {
  const hay = [
    s.name,
    s.stack,
    s.server_name,
    s.state,
    ...s.public.map((p) => p.url),
  ]
    .join(" ")
    .toLowerCase();
  return terms.every((t) => hay.includes(t));
};

export default function Services() {
  useSetTitle("Services");
  const [server, setServer] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const servers = useRead("ListServers", {}).data ?? [];
  const services =
    useRead(
      "ListServices",
      { server: server ?? undefined },
      { refetchInterval: 10_000 },
    ).data ?? [];
  const terms = search.toLowerCase().split(" ").filter(Boolean);
  const rows = useMemo(
    () => services.filter((s) => matches(s, terms)),
    [services, search],
  );

  return (
    <Page
      title="Services"
      icon={ICONS.Deployment}
      description="Everything running on your servers. Make any of it public with one click."
    >
      <Stack>
        <Group justify="space-between">
          <Select
            placeholder="All servers"
            clearable
            value={server}
            onChange={setServer}
            data={servers.map((s) => ({ value: s.id, label: s.name }))}
            w={{ base: "100%", xs: 220 }}
          />
          <SearchInput value={search} onSearch={setSearch} />
        </Group>
        <DataTable
          data={rows}
          tableKey="services-page-v1"
          columns={[
            {
              id: "Service",
              accessorFn: (s) => `${s.stack} ${s.name}`,
              header: ({ column }) => (
                <SortableHeader column={column} title="Service" />
              ),
              cell: ({ row: { original: s } }) =>
                s.target.type === "Deployment" ? (
                  <ResourceLink type="Deployment" id={s.target.id} />
                ) : (
                  <Group gap={4} wrap="nowrap">
                    <ResourceLink type="Stack" id={s.target.id} />
                    <Text c="dimmed">/</Text>
                    <Text>{s.name}</Text>
                  </Group>
                ),
            },
            {
              id: "Server",
              accessorKey: "server_name",
              header: ({ column }) => (
                <SortableHeader column={column} title="Server" />
              ),
              cell: ({ row: { original: s } }) =>
                s.server_id ? (
                  <ResourceLink type="Server" id={s.server_id} />
                ) : (
                  <Text c="dimmed">-</Text>
                ),
            },
            {
              id: "State",
              accessorKey: "state",
              header: ({ column }) => (
                <SortableHeader column={column} title="State" />
              ),
              cell: ({ row: { original: s } }) => (
                <StatusBadge text={s.state} intent={stateIntent(s.state)} />
              ),
            },
            {
              header: "Ports",
              cell: ({ row: { original: s } }) =>
                s.ports.length ? (
                  <Text fz="sm">
                    {s.ports.map((p) => `${p.host}->${p.container}`).join(", ")}
                  </Text>
                ) : (
                  <Text c="dimmed">-</Text>
                ),
            },
            {
              header: "LAN URL",
              cell: ({ row: { original: s } }) => (
                <Links
                  urls={s.ports.flatMap((p) => (p.lan_url ? [p.lan_url] : []))}
                />
              ),
            },
            {
              header: "Public URL",
              cell: ({ row: { original: s } }) => (
                <Stack gap={2}>
                  <Links urls={s.public.map((p) => p.url)} />
                  {s.public
                    .filter((p) => p.status !== "ready")
                    .map((p) => (
                      <StatusBadge
                        key={p.domain_id}
                        text={p.status}
                        intent={domainStatusIntention(p.status)}
                      />
                    ))}
                </Stack>
              ),
            },
            {
              header: "",
              id: "actions",
              cell: ({ row: { original: s } }) => <ServiceAccessButton item={s} />,
            },
          ]}
        />
      </Stack>
    </Page>
  );
}
