import { usePermissions, useRead, useWrite } from "@/lib/hooks";
import { Config, ConfigInput, ConfigItem } from "mogh_ui";
import { useLocalStorage } from "@mantine/hooks";
import { Select, Stack } from "@mantine/core";
import { Types } from "komodo_client";
import ResourceSelector from "@/resources/selector";
import { useFullDomain } from ".";

const PROXIED_OPTIONS = [
  { value: "default", label: "Provider default" },
  { value: "true", label: "Proxied" },
  { value: "false", label: "DNS only" },
];

/** Account names usable for a layer. Always includes the current value. */
function useAccountOptions(layer: Types.DomainLayer, current: string) {
  const accounts = useRead("ListDomainAccounts", {}).data;
  const names = (accounts ?? [])
    .filter((a) => a.capabilities.includes(layer))
    .map((a) => a.name);
  if (current && !names.includes(current)) names.push(current);
  return { names, accounts };
}

function AccountSelect({
  layer,
  value,
  onChange,
  disabled,
}: {
  layer: Types.DomainLayer;
  value: string;
  onChange: (account: string) => void;
  disabled: boolean;
}) {
  const { names } = useAccountOptions(layer, value);
  return (
    <ConfigItem
      label="Account"
      description="Domain account used for this layer. Leave empty to not manage it."
    >
      <Select
        value={value || null}
        onChange={(account) => onChange(account ?? "")}
        data={names}
        placeholder="Not managed"
        disabled={disabled}
        clearable
        searchable
        w={{ base: "85%", lg: 400 }}
      />
    </ConfigItem>
  );
}

function DnsFields({
  dns,
  set,
  disabled,
}: {
  dns: Types.DnsBinding;
  set: (dns: Types.DnsBinding) => void;
  disabled: boolean;
}) {
  return (
    <Stack>
      <AccountSelect
        layer={Types.DomainLayer.Dns}
        value={dns.account ?? ""}
        onChange={(account) => set({ ...dns, account })}
        disabled={disabled}
      />
      {!!dns.account && (
        <>
          <ConfigItem label="Record Type">
            <Select
              value={dns.record_type}
              onChange={(record_type) =>
                record_type &&
                set({ ...dns, record_type: record_type as Types.DnsRecordType })
              }
              data={Object.values(Types.DnsRecordType)}
              disabled={disabled}
              w={{ base: "85%", lg: 200 }}
            />
          </ConfigItem>
          <ConfigInput
            label="Content"
            description="Record content. Leave empty to point at the ingress tunnel."
            placeholder="Auto (tunnel)"
            value={dns.content}
            onValueChange={(content) => set({ ...dns, content })}
            disabled={disabled}
          />
          <ConfigInput
            label="TTL"
            description="Seconds. 0 = automatic."
            value={dns.ttl}
            onValueChange={(v) =>
              set({ ...dns, ttl: Math.max(0, parseInt(v) || 0) })
            }
            disabled={disabled}
          />
          <ConfigItem
            label="Proxied"
            description="Whether traffic is proxied through the provider."
          >
            <Select
              value={dns.proxied === undefined ? "default" : String(dns.proxied)}
              onChange={(v) =>
                set({
                  ...dns,
                  proxied: v === "true" ? true : v === "false" ? false : undefined,
                })
              }
              data={PROXIED_OPTIONS}
              allowDeselect={false}
              disabled={disabled}
              w={{ base: "85%", lg: 200 }}
            />
          </ConfigItem>
        </>
      )}
    </Stack>
  );
}

function IngressFields({
  ingress,
  set,
  disabled,
}: {
  ingress: Types.IngressBinding;
  set: (ingress: Types.IngressBinding) => void;
  disabled: boolean;
}) {
  const { accounts } = useAccountOptions(
    Types.DomainLayer.Ingress,
    ingress.account ?? "",
  );
  const tunnels: string[] = (
    accounts?.find((a) => a.name === ingress.account)?.params?.tunnels ?? []
  ).map((t: { name: string }) => t.name);
  if (ingress.tunnel && !tunnels.includes(ingress.tunnel)) {
    tunnels.push(ingress.tunnel);
  }

  return (
    <Stack>
      <AccountSelect
        layer={Types.DomainLayer.Ingress}
        value={ingress.account ?? ""}
        onChange={(account) => set({ ...ingress, account })}
        disabled={disabled}
      />
      {!!ingress.account && (
        <>
          <ConfigItem
            label="Tunnel"
            description="Tunnel registered on the account. Leave empty to pick automatically from the bound service's Server."
          >
            <Select
              value={ingress.tunnel || null}
              onChange={(tunnel) => set({ ...ingress, tunnel: tunnel ?? "" })}
              data={tunnels}
              placeholder="Auto"
              disabled={disabled}
              clearable
              w={{ base: "85%", lg: 400 }}
            />
          </ConfigItem>
          <ConfigInput
            label="Service"
            description="Origin service, eg. http://localhost:8080"
            placeholder="http://localhost:8080"
            value={ingress.service}
            onValueChange={(service) => set({ ...ingress, service })}
            disabled={disabled}
          />
        </>
      )}
    </Stack>
  );
}

function ServiceFields({
  config,
  setTarget,
  setUpdate,
  disabled,
}: {
  config: Types.DomainConfig;
  setTarget: (target: Types.ResourceTarget | undefined) => void;
  setUpdate: (u: Partial<Types.DomainConfig>) => void;
  disabled: boolean;
}) {
  const type = config.target?.type;
  const stack = type === "Stack";
  return (
    <Stack>
      <ConfigItem label="Target">
        <Select
          value={type ?? null}
          onChange={(t) =>
            setTarget(t ? ({ type: t, id: "" } as Types.ResourceTarget) : undefined)
          }
          data={["Deployment", "Stack"]}
          placeholder="None"
          disabled={disabled}
          clearable
          w={{ base: "85%", lg: 200 }}
        />
        {(type === "Deployment" || type === "Stack") && (
          <ResourceSelector
            type={type}
            selected={config.target?.id}
            onSelect={(id) => setTarget({ type, id })}
            disabled={disabled}
            clearable={false}
          />
        )}
      </ConfigItem>
      {stack && (
        <ConfigInput
          label="Service"
          description="Compose service name."
          value={config.service}
          onValueChange={(service) => setUpdate({ service })}
          disabled={disabled}
        />
      )}
      {!!type && (
        <ConfigInput
          label="Port"
          description="Published port of the service (used for the link in DNS mode)."
          value={config.port}
          onValueChange={(v) =>
            setUpdate({ port: Math.min(65535, Math.max(0, parseInt(v) || 0)) })
          }
          disabled={disabled}
        />
      )}
    </Stack>
  );
}

export default function DomainConfig({ id }: { id: string }) {
  const { canWrite } = usePermissions({ type: "Domain", id });
  const config = useFullDomain(id)?.config;
  const global_disabled =
    useRead("GetCoreInfo", {}).data?.ui_write_disabled ?? false;
  const { mutateAsync } = useWrite("UpdateDomain");
  const [update, setUpdate] = useLocalStorage<Partial<Types.DomainConfig>>({
    key: `domain-${id}-update-v1`,
    defaultValue: {},
  });

  if (!config) return null;
  const disabled = global_disabled || !canWrite;

  return (
    <Config
      disabled={disabled}
      original={config}
      update={update}
      setUpdate={setUpdate}
      onSave={() => mutateAsync({ id, config: update })}
      groups={{
        "": [
          {
            label: "Hostname",
            labelHidden: true,
            fields: {
              hostname: {
                label: "Hostname",
                description:
                  "Full public hostname, eg. app.example.com. The DNS zone is detected automatically.",
                placeholder: "app.example.com",
              },
            },
          },
          {
            label: "Service",
            description:
              "The Deployment / Stack this hostname serves. Prefer Add domain on the service page.",
            fields: {
              target: (_target, set) => (
                <ServiceFields
                  config={{ ...config, ...update }}
                  setTarget={(target) => set({ target })}
                  setUpdate={(u) => setUpdate({ ...update, ...u })}
                  disabled={disabled}
                />
              ),
            },
          },
          {
            label: "Advanced: DNS record",
            description: "One DNS record for the hostname.",
            fields: {
              dns: (dns, set) => (
                <DnsFields
                  dns={dns!}
                  set={(dns) => set({ dns })}
                  disabled={disabled}
                />
              ),
            },
          },
          {
            label: "Advanced: Tunnel rule",
            description: "One tunnel ingress rule for the hostname.",
            fields: {
              ingress: (ingress, set) => (
                <IngressFields
                  ingress={ingress!}
                  set={(ingress) => set({ ingress })}
                  disabled={disabled}
                />
              ),
            },
          },
          {
            label: "On delete",
            labelHidden: true,
            fields: {
              on_delete: {
                label: "On Delete",
                description:
                  "Cleanup removes remote objects created by Komodo when this Domain is deleted. Retain leaves them untouched.",
                options: Object.values(Types.OnDelete).map((value) => ({
                  value,
                })),
              },
            },
          },
        ],
      }}
    />
  );
}
