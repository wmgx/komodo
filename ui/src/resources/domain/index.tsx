import { useEffect, useState } from "react";
import { displayHostname } from "./hostname";
import ExportToml from "@/components/export-toml";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import {
  Alert,
  Badge,
  Button,
  Checkbox,
  Code,
  Group,
  NumberInput,
  Paper,
  Select,
  Stack,
  Table,
  Text,
  Textarea,
  TextInput,
  Title,
} from "@mantine/core";
import { useQueryClient } from "@tanstack/react-query";
import { Types } from "komodo_client";
import {
  komodo_client,
  useRead,
  usePermissions,
  useSetTitle,
  useUser,
} from "@/lib/hooks";

type EditorConfig = Required<Types.DomainConfig>;
const normalizeConfig = (
  config: Types.DomainConfig = emptyConfig(),
): EditorConfig => ({
  ...emptyConfig(),
  ...config,
  dns: config.dns ?? { type: "External" },
  tls: config.tls ?? { mode: Types.TlsMode.RequireHttps, profile: "" },
  reconcile_mode: config.reconcile_mode ?? Types.ReconcileMode.Manual,
});
const emptyConfig = (): EditorConfig => ({
  hostname: "",
  target: { type: "Url", params: { url: "" } },
  dns: { type: "External" },
  ingress: "",
  tls: { mode: Types.TlsMode.RequireHttps, profile: "" },
  reconcile_mode: Types.ReconcileMode.Manual,
});
const idOf = (resource: { _id?: { $oid: string } }) => resource._id?.$oid ?? "";
// Only display the API's structured public error fields. Never print raw HTTP errors.
function message(error: unknown): string {
  const e = error as {
    result?: { error?: string | Types.DomainError };
    message?: string;
  };
  const detail = e?.result?.error;
  return typeof detail === "object"
    ? `${detail.code}: ${detail.message}`
    : typeof detail === "string"
      ? detail
      : "Request failed. Check permissions, connection configuration, and retry.";
}
function download(name: string, value: unknown) {
  const url = URL.createObjectURL(
    new Blob([JSON.stringify(value, null, 2)], { type: "application/json" }),
  );
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

export default function Domains({
  connections,
}: {
  connections?: "DnsProvider" | "Ingress";
}) {
  useSetTitle("Domains");
  const info = useRead("GetCoreInfo", {});
  if (!info.data) return <Text>Loading Core capabilities…</Text>;
  if (!info.data.domain_management_enabled)
    return (
      <Alert title="Domain management is disabled">
        An administrator must explicitly enable domain management in Core
        configuration.
      </Alert>
    );
  return (
    <Stack>
      <Title order={2}>Domains</Title>
      <Group>
        <Button component={Link} to="/domains" variant="default">
          Bindings
        </Button>
        <Button component={Link} to="/dns-providers" variant="default">
          DNS connections
        </Button>
        <Button component={Link} to="/ingresses" variant="default">
          Ingress connections
        </Button>
      </Group>
      {connections ? (
        <Connections key={connections} kind={connections} />
      ) : (
        <Bindings />
      )}
    </Stack>
  );
}

function Bindings() {
  const { id } = useParams();
  const navigate = useNavigate();
  const domains = useRead("ListDomains", {});
  const { canCreate } = usePermissions({ type: "Domain", id: "" });
  const [filter, setFilter] = useState("");
  return (
    <Stack>
      <Group>
        <TextInput
          placeholder="Filter bindings by name, hostname or tag"
          value={filter}
          onChange={(e) => setFilter(e.currentTarget.value)}
        />
        <Button disabled={!canCreate} onClick={() => navigate("/domains/new")}>
          New binding
        </Button>
      </Group>
      {!!domains.error && <Alert color="red">{message(domains.error)}</Alert>}
      <Table>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>Name</Table.Th>
            <Table.Th>Hostname</Table.Th>
            <Table.Th>State</Table.Th>
            <Table.Th>Generation</Table.Th>
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {domains.data
            ?.filter((d) =>
              `${d.name} ${displayHostname(d)} ${d.config?.hostname} ${(d.tags ?? []).join(" ")}`
                .toLowerCase()
                .includes(filter.toLowerCase()),
            )
            .map((d) => (
              <Table.Tr key={idOf(d)}>
                <Table.Td>
                  <Link to={`/domains/${idOf(d)}`}>{d.name}</Link>
                </Table.Td>
                <Table.Td>
                  <Text>{displayHostname(d)}</Text>
                  {displayHostname(d) !== d.config?.hostname && (
                    <Text size="xs" c="dimmed">
                      {d.config?.hostname}
                    </Text>
                  )}
                </Table.Td>
                <Table.Td>{d.info?.state}</Table.Td>
                <Table.Td>
                  {d.info?.observed_generation} / {d.info?.generation}
                </Table.Td>
              </Table.Tr>
            ))}
        </Table.Tbody>
      </Table>
      {id && <BindingEditor key={id} id={id} />}
    </Stack>
  );
}

function BindingEditor({ id }: { id: string }) {
  const fresh = id === "new";
  const { canWrite, canExecute, canCreate } = usePermissions({
    type: "Domain",
    id: fresh ? "" : id,
  });
  const canEdit = fresh ? canCreate : canWrite;
  const client = komodo_client();
  const qc = useQueryClient();
  const nav = useNavigate();
  const domain = useRead("GetDomain", { domain: id }, { enabled: !fresh });
  const status = useRead(
    "GetDomainStatus",
    { domain: id },
    { enabled: !fresh, refetchInterval: 3000 },
  );
  const providers = useRead("ListDnsProviders", {});
  const ingresses = useRead("ListIngresses", {});
  const adapters = useRead("ListDomainAdapters", {});
  const deployments = useRead("ListDeployments", {});
  const stacks = useRead("ListStacks", {});
  const [name, setName] = useState("");
  const [config, setConfig] = useState<EditorConfig>(emptyConfig);
  const [plan, setPlan] = useState<Types.DomainPlan>();
  const [confirmed, setConfirmed] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [accepted, setAccepted] = useState("");
  const [migrationIngress, setMigrationIngress] = useState<string | null>(null);
  const [migrationProvider, setMigrationProvider] = useState<string | null>(
    null,
  );
  const [objects, setObjects] = useState("");
  useEffect(() => {
    if (domain.data) {
      setName(domain.data.name);
      setConfig({
        ...normalizeConfig(domain.data.config),
        hostname: displayHostname(domain.data),
      });
    }
  }, [domain.data]);
  const ingress = ingresses.data?.find(
    (i) => idOf(i) === config.ingress || i.name === config.ingress,
  );
  const capability = adapters.data?.find(
    (a) => a.adapter === ingress?.config?.adapter.type,
  );
  const patch = (value: Partial<EditorConfig>) => {
    setConfig((c) => ({ ...c, ...value }));
    setPlan(undefined);
    setConfirmed(false);
  };
  function patchTransport(value: {
    port?: number;
    scheme?: Types.TargetScheme;
  }) {
    const target = config.target;
    if (target.type === "Deployment")
      patch({
        target: { type: "Deployment", params: { ...target.params, ...value } },
      });
    if (target.type === "StackService")
      patch({
        target: {
          type: "StackService",
          params: { ...target.params, ...value },
        },
      });
  }
  async function run(action: () => Promise<void>) {
    setPending(true);
    setError("");
    try {
      await action();
      await qc.invalidateQueries();
    } catch (e) {
      setError(message(e));
    } finally {
      setPending(false);
    }
  }
  async function preview(intent: Types.DomainChangeIntent) {
    await run(async () => {
      setPlan(await client.read("PlanDomainChange", { intent }));
      setConfirmed(false);
    });
  }
  async function apply() {
    if (!plan || !confirmed) return;
    await run(async () => {
      const request = {
        domain: plan.domain_id ?? plan.name,
        plan_id: plan.plan_id,
        expected_generation: plan.generation,
        request_id: plan.request_id,
      };
      const update = await client.execute(
        plan.operation === "Remove"
          ? "RemoveDomain"
          : plan.operation === "Migrate"
            ? "MigrateDomain"
            : "ReconcileDomain",
        request,
      );
      setAccepted(update._id?.$oid ?? "accepted");
      setPlan(undefined);
      setConfirmed(false);
      if (fresh)
        nav(
          `/domains/${update.target.type === "Domain" ? update.target.id : (plan.domain_id ?? plan.name)}`,
        );
    });
  }
  async function exportConfig(): Promise<Types.DomainConfig> {
    const saved = normalizeConfig(
      (await client.read("GetDomain", { domain: id })).config,
    );
    const target: Types.DomainTarget =
      saved.target.type === "Deployment"
        ? {
            ...saved.target,
            params: {
              ...saved.target.params,
              id: (
                await client.read("GetDeployment", {
                  deployment: saved.target.params.id,
                })
              ).name,
            },
          }
        : saved.target.type === "StackService"
          ? {
              ...saved.target,
              params: {
                ...saved.target.params,
                stack_id: (
                  await client.read("GetStack", {
                    stack: saved.target.params.stack_id,
                  })
                ).name,
              },
            }
          : saved.target;
    const dns: Types.DomainDns =
      saved.dns.type === "Managed"
        ? {
            ...saved.dns,
            params: {
              ...saved.dns.params,
              provider: (
                await client.read("GetDnsProvider", {
                  dns_provider: saved.dns.params.provider,
                })
              ).name,
            },
          }
        : saved.dns;
    return {
      ...saved,
      target,
      dns,
      ingress: (await client.read("GetIngress", { ingress: saved.ingress }))
        .name,
    };
  }
  if (!fresh && !domain.data)
    return (
      <Alert>{domain.error ? message(domain.error) : "Loading binding…"}</Alert>
    );
  return (
    <Paper withBorder p="md">
      <Stack>
        <Title order={3}>{fresh ? "New binding" : name}</Title>
        <Text size="sm" c="dimmed">
          Saving changes only stores desired configuration. Preview and
          explicitly apply a server plan to change DNS or routes.
        </Text>
        {error && <Alert color="red">{error}</Alert>}
        {!!(providers.error || ingresses.error || adapters.error) && (
          <Alert color="red">
            {message(providers.error || ingresses.error || adapters.error)}
          </Alert>
        )}
        {accepted && (
          <Alert title="Operation accepted">
            Update {accepted}. Acceptance does not mean the site is ready;
            status below is observed separately.
          </Alert>
        )}
        <TextInput
          label="Name"
          value={name}
          disabled={!fresh}
          onChange={(e) => {
            setName(e.currentTarget.value);
            setPlan(undefined);
            setConfirmed(false);
          }}
        />
        <TextInput
          label="Hostname"
          description={
            !fresh
              ? `Normalized DNS name: ${domain.data?.config?.hostname ?? ""}`
              : undefined
          }
          placeholder="app.example.com"
          value={config.hostname}
          onChange={(e) => patch({ hostname: e.currentTarget.value })}
        />
        <Select
          label="Ingress"
          value={config.ingress}
          data={
            ingresses.data?.map((i) => ({ value: idOf(i), label: i.name })) ??
            []
          }
          onChange={(v) => patch({ ingress: v ?? "" })}
        />
        <Select
          label="Target type"
          value={config.target.type}
          data={capability?.target_types ?? []}
          description={
            !capability
              ? "Choose an ingress to load supported target types"
              : undefined
          }
          onChange={(v) => {
            if (v === "Url")
              patch({ target: { type: "Url", params: { url: "" } } });
            if (v === "Deployment")
              patch({
                target: {
                  type: "Deployment",
                  params: { id: "", port: 80, scheme: Types.TargetScheme.Http },
                },
              });
            if (v === "StackService")
              patch({
                target: {
                  type: "StackService",
                  params: {
                    stack_id: "",
                    service: "",
                    port: 80,
                    scheme: Types.TargetScheme.Http,
                  },
                },
              });
          }}
        />
        {config.target.type === "Url" && (
          <TextInput
            label="Backend URL"
            description="Must fall within administrator-authorized network ranges."
            value={config.target.params.url}
            onChange={(e) =>
              patch({
                target: { type: "Url", params: { url: e.currentTarget.value } },
              })
            }
          />
        )}
        {config.target.type === "Deployment" && (
          <Select
            label="Deployment"
            value={config.target.params.id}
            data={
              deployments.data?.map((d) => ({ value: d.id, label: d.name })) ??
              []
            }
            onChange={(v) => {
              if (config.target.type === "Deployment")
                patch({
                  target: {
                    ...config.target,
                    params: { ...config.target.params, id: v ?? "" },
                  },
                });
            }}
          />
        )}
        {config.target.type === "StackService" && (
          <>
            <Select
              label="Stack"
              value={config.target.params.stack_id}
              data={
                stacks.data?.map((d) => ({ value: d.id, label: d.name })) ?? []
              }
              onChange={(v) => {
                if (config.target.type === "StackService")
                  patch({
                    target: {
                      ...config.target,
                      params: { ...config.target.params, stack_id: v ?? "" },
                    },
                  });
              }}
            />
            <TextInput
              label="Service"
              value={config.target.params.service}
              onChange={(e) => {
                if (config.target.type === "StackService")
                  patch({
                    target: {
                      ...config.target,
                      params: {
                        ...config.target.params,
                        service: e.currentTarget.value,
                      },
                    },
                  });
              }}
            />
          </>
        )}
        {config.target.type !== "Url" && (
          <Group grow>
            <NumberInput
              label="Application port"
              min={1}
              max={65535}
              value={config.target.params.port}
              onChange={(v) => patchTransport({ port: Number(v) })}
            />
            <Select
              label="Application protocol"
              value={config.target.params.scheme}
              data={Object.values(Types.TargetScheme)}
              onChange={(v) =>
                patchTransport({ scheme: v as Types.TargetScheme })
              }
            />
          </Group>
        )}
        <Select
          label="DNS"
          value={
            config.dns.type === "External"
              ? "external"
              : config.dns.params.provider
          }
          data={[
            { value: "external", label: "External — managed outside Komodo" },
            ...(providers.data?.map((p) => ({
              value: idOf(p),
              label: p.name,
            })) ?? []),
          ]}
          onChange={(v) =>
            patch({
              dns:
                v === "external"
                  ? { type: "External" }
                  : {
                      type: "Managed",
                      params: {
                        provider: v ?? "",
                        ttl: 1,
                        proxy_mode: Types.ProxyMode.DnsOnly,
                      },
                    },
            })
          }
        />
        {config.dns.type === "Managed" && (
          <Group grow>
            <NumberInput
              label="DNS TTL (1 = automatic)"
              min={1}
              value={config.dns.params.ttl}
              onChange={(v) => {
                if (config.dns.type === "Managed")
                  patch({
                    dns: {
                      ...config.dns,
                      params: { ...config.dns.params, ttl: Number(v) },
                    },
                  });
              }}
            />
            <Select
              label="Proxy mode"
              value={config.dns.params.proxy_mode}
              data={
                adapters.data?.find(
                  (a) =>
                    a.adapter ===
                    providers.data?.find(
                      (p) =>
                        idOf(p) ===
                        (config.dns.type === "Managed"
                          ? config.dns.params.provider
                          : ""),
                    )?.config?.adapter,
                )?.proxy_modes ?? []
              }
              onChange={(v) => {
                if (config.dns.type === "Managed")
                  patch({
                    dns: {
                      ...config.dns,
                      params: {
                        ...config.dns.params,
                        proxy_mode: v as Types.ProxyMode,
                      },
                    },
                  });
              }}
            />
          </Group>
        )}
        <Select
          label="HTTPS policy"
          value={config.tls.mode}
          data={capability?.tls_modes ?? []}
          onChange={(v) =>
            patch({ tls: { ...config.tls, mode: v as Types.TlsMode } })
          }
        />
        <Select
          label="TLS profile"
          value={config.tls.profile}
          data={
            ingress?.config?.tls_profiles
              ?.filter((p) => p.mode === config.tls.mode)
              .map((p) => p.name) ?? []
          }
          onChange={(v) => patch({ tls: { ...config.tls, profile: v ?? "" } })}
        />
        <Select
          label="Reconciliation"
          value={config.reconcile_mode}
          data={Object.values(Types.ReconcileMode)}
          onChange={(v) => patch({ reconcile_mode: v as Types.ReconcileMode })}
        />
        <Group>
          <Button
            disabled={!canEdit}
            loading={pending}
            onClick={() =>
              run(async () => {
                const saved = fresh
                  ? await client.write("CreateDomain", { name, config })
                  : await client.write("UpdateDomain", {
                      id,
                      config,
                      expected_generation: domain.data!.info!.generation,
                    });
                nav(`/domains/${idOf(saved)}`);
              })
            }
          >
            Save desired configuration
          </Button>
          <Button
            disabled={!canEdit}
            loading={pending}
            variant="default"
            onClick={() =>
              preview(
                fresh
                  ? { type: "Create", params: { name, config } }
                  : {
                      type: "Update",
                      params: {
                        domain: id,
                        config,
                        expected_generation: domain.data!.info!.generation,
                      },
                    },
              )
            }
          >
            Preview changes
          </Button>
          {!fresh && (
            <>
              <Button
                disabled={!canExecute}
                variant="default"
                onClick={() =>
                  preview({ type: "Reconcile", params: { domain: id } })
                }
              >
                Plan reconciliation
              </Button>
              <Button
                variant="default"
                onClick={() =>
                  run(async () =>
                    download(`${name}.json`, {
                      name,
                      config: await exportConfig(),
                    }),
                  )
                }
              >
                Export desired configuration
              </Button>
              <ExportToml targets={[{ type: "Domain", id }]} />
            </>
          )}
        </Group>
        {status.data && (
          <>
            <Title order={4}>Observed status</Title>
            <Group>
              <Badge>{status.data.state}</Badge>
              <Text>
                Observed generation {status.data.observed_generation} /{" "}
                {status.data.generation}
              </Text>
            </Group>
            {status.data.error && (
              <Alert color="red">
                {status.data.error.code}: {status.data.error.message}
              </Alert>
            )}
            <Table>
              <Table.Tbody>
                {status.data.conditions.map((c) => (
                  <Table.Tr key={c.condition}>
                    <Table.Td>{c.condition}</Table.Td>
                    <Table.Td>{c.status}</Table.Td>
                    <Table.Td>
                      {c.reason}: {c.message}
                    </Table.Td>
                    <Table.Td>
                      {new Date(c.observed_at).toLocaleString()}
                    </Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          </>
        )}
        {!!status.error && <Alert color="red">{message(status.error)}</Alert>}
        {!fresh && canWrite && (
          <>
            <Title order={4}>Migration and ownership</Title>
            <Group grow>
              <Select
                clearable
                label="Replacement DNS connection"
                value={migrationProvider}
                data={
                  providers.data?.map((p) => ({
                    value: idOf(p),
                    label: p.name,
                  })) ?? []
                }
                onChange={setMigrationProvider}
              />
              <Select
                clearable
                label="Replacement ingress"
                value={migrationIngress}
                data={
                  ingresses.data?.map((p) => ({
                    value: idOf(p),
                    label: p.name,
                  })) ?? []
                }
                onChange={setMigrationIngress}
              />
            </Group>
            <Group>
              <Button
                variant="default"
                disabled={!migrationProvider && !migrationIngress}
                onClick={() =>
                  preview({
                    type: "Migrate",
                    params: {
                      domain: id,
                      dns_provider: migrationProvider ?? undefined,
                      ingress: migrationIngress ?? undefined,
                    },
                  })
                }
              >
                Plan migration
              </Button>
              <Button
                variant="default"
                onClick={() => {
                  if (
                    window.confirm(
                      "Finalize migration and clean up the old owned resources?",
                    )
                  )
                    run(async () => {
                      const update = await client.execute(
                        "FinalizeDomainMigration",
                        {
                          domain: id,
                          plan_id: "",
                          expected_generation: domain.data!.info!.generation,
                          request_id: crypto.randomUUID(),
                        },
                      );
                      setAccepted(update._id?.$oid ?? "accepted");
                    });
                }}
              >
                Finalize migration
              </Button>
            </Group>
            <TextInput
              label="Exact object IDs to adopt (one per line or comma-separated)"
              value={objects}
              onChange={(e) => setObjects(e.currentTarget.value)}
            />
            <Group>
              <Button
                variant="default"
                disabled={!objects.trim()}
                onClick={() =>
                  preview({
                    type: "Adopt",
                    params: {
                      domain: id,
                      objects: objects
                        .split(/[\n,]/)
                        .map((s) => s.trim())
                        .filter(Boolean),
                    },
                  })
                }
              >
                Plan explicit adoption
              </Button>
              <Button
                color="orange"
                variant="default"
                onClick={() =>
                  preview({ type: "Abandon", params: { domain: id } })
                }
              >
                Plan abandon (keep external objects)
              </Button>
              <Button
                color="red"
                variant="default"
                onClick={() =>
                  preview({ type: "Remove", params: { domain: id } })
                }
              >
                Plan removal
              </Button>
              <Button
                color="red"
                variant="subtle"
                loading={pending}
                onClick={() => {
                  if (
                    !window.confirm(
                      "Delete the binding record? Core will reject this until all owned external resources have been cleaned or explicitly abandoned.",
                    )
                  )
                    return;
                  run(async () => {
                    await client.write("DeleteDomain", { id });
                    nav("/domains");
                  });
                }}
              >
                Delete cleaned binding record
              </Button>
            </Group>
          </>
        )}
        {!fresh && <DomainPermissions type="Domain" id={idOf(domain.data!)} />}
        {plan && (
          <Paper withBorder p="md">
            <Stack>
              <Title order={4}>{plan.operation} plan</Title>
              <Text>
                Generation {plan.generation} · expires{" "}
                {new Date(plan.expires_at).toLocaleString()}
              </Text>
              {plan.requires_downtime && (
                <Alert color="orange">This migration requires downtime.</Alert>
              )}
              {plan.warnings.map((w, i) => (
                <Alert color="orange" key={i}>
                  {w}
                </Alert>
              ))}
              <ol>
                {plan.steps.map((s, i) => (
                  <li key={i}>{s}</li>
                ))}
              </ol>
              <Checkbox
                label="I reviewed the plan and authorize these changes."
                checked={confirmed}
                onChange={(e) => setConfirmed(e.currentTarget.checked)}
              />
              <Group>
                <Button
                  disabled={
                    !confirmed ||
                    Date.now() >= plan.expires_at ||
                    (plan.operation === "Reconcile" ? !canExecute : !canEdit)
                  }
                  loading={pending}
                  onClick={apply}
                >
                  Apply server plan
                </Button>
                <Button
                  variant="default"
                  onClick={() => download(`${name}.plan.json`, plan)}
                >
                  Download plan reference
                </Button>
              </Group>
            </Stack>
          </Paper>
        )}
      </Stack>
    </Paper>
  );
}

function Connections({ kind }: { kind: "DnsProvider" | "Ingress" }) {
  const [searchParams] = useSearchParams();
  const admin = useUser().data?.admin;
  const client = komodo_client();
  const qc = useQueryClient();
  const dns = useRead("ListDnsProviders", {});
  const ingress = useRead("ListIngresses", {});
  const adapters = useRead("ListDomainAdapters", {});
  const [selected, setSelected] = useState(searchParams.get("selected") ?? "");
  const [name, setName] = useState("");
  const [json, setJson] = useState("{}");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [validation, setValidation] =
    useState<Types.DomainConnectionValidation>();
  const list = kind === "DnsProvider" ? dns.data : ingress.data;
  const current = list?.find((r) => idOf(r) === selected);
  useEffect(() => {
    if (current) {
      setName(current.name);
      setJson(JSON.stringify(current.config, null, 2));
    }
  }, [current]);
  async function run(action: () => Promise<void>) {
    setPending(true);
    setError("");
    try {
      await action();
      await qc.invalidateQueries();
    } catch (e) {
      setError(
        e instanceof SyntaxError ? "Invalid JSON configuration." : message(e),
      );
    } finally {
      setPending(false);
    }
  }
  function choose(id: string) {
    const resource = list?.find((r) => idOf(r) === id);
    setSelected(id);
    setName(resource?.name ?? "");
    setJson(JSON.stringify(resource?.config ?? {}, null, 2));
    setValidation(undefined);
  }
  return (
    <Stack>
      <Title order={3}>
        {kind === "DnsProvider" ? "DNS connections" : "Ingress connections"}
      </Title>
      <Text>
        Connections store references to administrator-defined Core secrets.
        Never paste an API token into this form. Saving does not install a proxy
        or create a Tunnel.
      </Text>
      {error && <Alert color="red">{error}</Alert>}
      {!!(kind === "DnsProvider" ? dns.error : ingress.error) && (
        <Alert color="red">
          {message(kind === "DnsProvider" ? dns.error : ingress.error)}
        </Alert>
      )}
      <Group>
        <Select
          label="Connection"
          value={selected}
          data={list?.map((r) => ({ value: idOf(r), label: r.name })) ?? []}
          onChange={(v) => choose(v ?? "")}
        />
        {admin && <Button onClick={() => choose("")}>New connection</Button>}
      </Group>
      {admin && (
        <>
          <TextInput
            label="Name"
            disabled={!!selected}
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
          />
          <Select
            label="Initialize configuration for installed adapter"
            data={
              adapters.data
                ?.filter((a) =>
                  kind === "DnsProvider"
                    ? a.dns_types.length > 0
                    : a.target_types.length > 0,
                )
                .map((a) => a.adapter) ?? []
            }
            disabled={!!selected}
            onChange={(value) => {
              if (!value) return;
              const capability = adapters.data!.find(
                (a) => a.adapter === value,
              )!;
              const secret = { source: "CoreSecret", name: "" };
              setJson(
                JSON.stringify(
                  kind === "DnsProvider"
                    ? {
                        adapter: value,
                        schema_version: capability.schema_version,
                        account_id: "",
                        zone_id: "",
                        secret,
                        allowed_hostname_suffixes: [],
                      }
                    : {
                        schema_version: capability.schema_version,
                        adapter: {
                          type: value,
                          params:
                            value === "TraefikFileV1"
                              ? {
                                  server: "",
                                  entrypoint: "websecure",
                                  dns_target: "",
                                  dedicated: false,
                                }
                              : {
                                  account_id: "",
                                  tunnel_id: "",
                                  secret,
                                  dedicated: false,
                                },
                        },
                        allowed_hostname_suffixes: [],
                        allowed_networks: [],
                        docker_network: "",
                        tls_profiles: [],
                        drain_seconds: 30,
                      },
                  null,
                  2,
                ),
              );
            }}
          />
          <Textarea
            label="Versioned connection configuration (JSON)"
            autosize
            minRows={12}
            maxRows={35}
            value={json}
            onChange={(e) => setJson(e.currentTarget.value)}
            description="Only CoreSecret references are accepted. The server validates schema, network and hostname scopes, identity changes and dependencies."
          />
          <Group>
            <Button
              loading={pending}
              onClick={() => {
                if (!window.confirm("Save this connection configuration?"))
                  return;
                run(async () => {
                  const config = JSON.parse(json);
                  const saved =
                    kind === "DnsProvider"
                      ? selected
                        ? await client.write("UpdateDnsProvider", {
                            id: selected,
                            config,
                            expected_generation: current!.info!.generation,
                          })
                        : await client.write("CreateDnsProvider", {
                            name,
                            config,
                          })
                      : selected
                        ? await client.write("UpdateIngress", {
                            id: selected,
                            config,
                            expected_generation: current!.info!.generation,
                          })
                        : await client.write("CreateIngress", { name, config });
                  setSelected(idOf(saved));
                });
              }}
            >
              Save connection
            </Button>
            {selected && (
              <Button
                color="red"
                variant="default"
                loading={pending}
                onClick={() => {
                  if (
                    !window.confirm(
                      "Delete this connection? Referenced connections cannot be deleted.",
                    )
                  )
                    return;
                  run(async () => {
                    if (kind === "DnsProvider")
                      await client.write("DeleteDnsProvider", { id: selected });
                    else await client.write("DeleteIngress", { id: selected });
                    choose("");
                  });
                }}
              >
                Delete connection
              </Button>
            )}
          </Group>
        </>
      )}
      {!admin && current && (
        <Code block>{JSON.stringify(current.config, null, 2)}</Code>
      )}
      {selected && (
        <Button
          variant="default"
          loading={pending}
          onClick={() =>
            run(async () => {
              setValidation(
                await client.read("ValidateDomainConnection", {
                  connection: { type: kind, params: selected },
                }),
              );
            })
          }
        >
          Validate connection (read only)
        </Button>
      )}
      {selected && <DomainPermissions type={kind} id={selected} />}
      {validation && (
        <>
          <Alert color={validation.valid ? "green" : "orange"}>
            {validation.valid
              ? "Connection validation passed. This does not verify a deployed site's availability."
              : "Connection validation failed."}
          </Alert>
          {validation.errors.map((e, i) => (
            <Alert color="red" key={i}>
              {e.code}: {e.message}
            </Alert>
          ))}
          <Code block>{JSON.stringify(validation.capabilities, null, 2)}</Code>
        </>
      )}
    </Stack>
  );
}

function DomainPermissions({
  type,
  id,
}: {
  type: "Domain" | "DnsProvider" | "Ingress";
  id: string;
}) {
  const admin = useUser().data?.admin ?? false;
  const users = useRead("ListUsers", {}, { enabled: admin });
  const groups = useRead("ListUserGroups", {}, { enabled: admin });
  const [subject, setSubject] = useState<string | null>(null);
  const [level, setLevel] = useState<Types.PermissionLevel>(
    Types.PermissionLevel.None,
  );
  const [attach, setAttach] = useState(false);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const userTarget: Types.UserTarget = {
    type: subject?.startsWith("group:") ? "UserGroup" : "User",
    id: subject?.split(":")[1] ?? "",
  };
  const permissions = useRead(
    "ListUserTargetPermissions",
    { user_target: userTarget },
    { enabled: admin && !!subject },
  );
  useEffect(() => {
    const p = permissions.data?.find(
      (p) => p.resource_target.type === type && p.resource_target.id === id,
    );
    setLevel(p?.level ?? Types.PermissionLevel.None);
    setAttach(p?.specific?.includes(Types.SpecificPermission.Attach) ?? false);
  }, [permissions.data, type, id]);
  if (!admin) return null;
  return (
    <Paper withBorder p="md">
      <Stack>
        <Title order={4}>Resource permissions</Title>
        <Select
          searchable
          label="User or group"
          value={subject}
          data={[
            ...(users.data?.map((u) => ({
              value: `user:${idOf(u)}`,
              label: u.username,
            })) ?? []),
            ...(groups.data?.map((g) => ({
              value: `group:${idOf(g)}`,
              label: `Group: ${g.name}`,
            })) ?? []),
          ]}
          onChange={setSubject}
        />
        <Select
          label="Permission level"
          value={level}
          data={Object.values(Types.PermissionLevel)}
          onChange={(v) => setLevel(v as Types.PermissionLevel)}
        />
        {type !== "Domain" && (
          <Checkbox
            label="Attach: may use this connection for a binding (never grants access to token values)"
            checked={attach}
            onChange={(e) => setAttach(e.currentTarget.checked)}
          />
        )}
        {error && <Alert color="red">{error}</Alert>}
        <Button
          disabled={!subject || permissions.isPending}
          loading={saving}
          onClick={async () => {
            setSaving(true);
            setError("");
            try {
              await komodo_client().write("UpdatePermissionOnTarget", {
                user_target: userTarget,
                resource_target: { type, id },
                permission: {
                  level,
                  specific:
                    attach && type !== "Domain"
                      ? [Types.SpecificPermission.Attach]
                      : [],
                },
              });
              await permissions.refetch();
            } catch (e) {
              setError(message(e));
            } finally {
              setSaving(false);
            }
          }}
        >
          Save permission
        </Button>
      </Stack>
    </Paper>
  );
}
