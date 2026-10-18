import {
  Alert,
  Badge,
  Checkbox,
  Group,
  ScrollArea,
  Select,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { Types } from "komodo_client";

/** A tunnel kept in account params. Keyed by `tunnel_id`. */
export interface TunnelSel {
  account_id: string;
  name: string;
  server: string;
}

export type TunnelSelMap = Record<string, TunnelSel>;

/** Zones and tunnels picked from a discovery result. */
export default function DiscoveryFields({
  discovery,
  zones,
  setZones,
  tunnels,
  setTunnels,
  servers,
}: {
  discovery?: Types.DomainDiscovery;
  zones: string[];
  setZones: (zones: string[]) => void;
  tunnels: TunnelSelMap;
  setTunnels: (tunnels: TunnelSelMap) => void;
  servers: Types.ServerListItem[];
}) {
  // Saved selections stay visible even if the latest read no longer lists them.
  const zoneNames = [
    ...new Set([...(discovery?.zones ?? []), ...zones]),
  ].sort();
  const rows = [
    ...(discovery?.ingresses ?? []).map((i) => ({
      id: i.id,
      cfName: i.name,
      account_id: i.account_id ?? "",
      status: i.status,
    })),
    ...Object.entries(tunnels)
      .filter(([id]) => !discovery?.ingresses.some((i) => i.id === id))
      .map(([id, t]) => ({
        id,
        cfName: t.name,
        account_id: t.account_id,
        status: undefined,
      })),
  ];
  const serverData = servers.map((s) => ({ value: s.id, label: s.name }));

  const patch = (id: string, change: Partial<TunnelSel>) =>
    setTunnels({ ...tunnels, [id]: { ...tunnels[id], ...change } });
  const toggle = (id: string, on: boolean, cfName: string, acc: string) => {
    if (!on) {
      const { [id]: _removed, ...rest } = tunnels;
      setTunnels(rest);
      return;
    }
    setTunnels({
      ...tunnels,
      [id]: { account_id: acc, name: cfName, server: "" },
    });
  };

  return (
    <Stack gap="md">
      {discovery?.warnings.map((w) => (
        <Alert key={w} color="yellow" p="xs">
          {w}
        </Alert>
      ))}

      <Stack gap="xs">
        <Text size="sm" fw={500}>
          Zones
        </Text>
        <Text size="xs" c="dimmed">
          Checked zones form an allow-list. Check none to allow every zone the
          token can access.
        </Text>
        {zoneNames.length === 0 ? (
          <Text size="sm" c="dimmed">
            No zones listed. Read from the provider first.
          </Text>
        ) : (
          <ScrollArea.Autosize mah={160}>
            <Checkbox.Group value={zones} onChange={setZones}>
              <Stack gap={4}>
                {zoneNames.map((z) => (
                  <Checkbox key={z} value={z} label={z} />
                ))}
              </Stack>
            </Checkbox.Group>
          </ScrollArea.Autosize>
        )}
      </Stack>

      <Stack gap="xs">
        <Text size="sm" fw={500}>
          Tunnels
        </Text>
        <Text size="xs" c="dimmed">
          Enable the tunnels Domains may publish through, and optionally bind
          each one to the Server it runs on. Not needed for DNS only.
        </Text>
        {rows.length === 0 && (
          <Text size="sm" c="dimmed">
            No tunnels listed.
          </Text>
        )}
        {rows.map((row) => {
          const sel = tunnels[row.id];
          return (
            <Group key={row.id} align="end" wrap="nowrap">
              <Checkbox
                checked={!!sel}
                onChange={(e) =>
                  toggle(
                    row.id,
                    e.currentTarget.checked,
                    row.cfName,
                    row.account_id,
                  )
                }
                label={
                  <Group gap="xs">
                    <Text size="sm">{row.cfName}</Text>
                    {row.status && (
                      <Badge
                        size="xs"
                        variant="default"
                        color={row.status === "healthy" ? "green" : "gray"}
                      >
                        {row.status}
                      </Badge>
                    )}
                  </Group>
                }
                miw={160}
              />
              {sel && (
                <>
                  <TextInput
                    size="xs"
                    placeholder="Local name"
                    value={sel.name}
                    onChange={(e) =>
                      patch(row.id, { name: e.currentTarget.value })
                    }
                  />
                  <Select
                    size="xs"
                    placeholder="Server (optional)"
                    clearable
                    searchable
                    data={
                      sel.server && !servers.some((s) => s.id === sel.server)
                        ? [
                            ...serverData,
                            { value: sel.server, label: sel.server },
                          ]
                        : serverData
                    }
                    value={sel.server || null}
                    onChange={(v) => patch(row.id, { server: v ?? "" })}
                  />
                </>
              )}
            </Group>
          );
        })}
      </Stack>
    </Stack>
  );
}
