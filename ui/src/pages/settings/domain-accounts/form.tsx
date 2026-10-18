import { komodo_client, useInvalidate, useRead, useWrite } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import {
  Accordion,
  Alert,
  Button,
  Group,
  Modal,
  PasswordInput,
  Select,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { CircleCheckBig } from "lucide-react";
import { MonacoEditor } from "mogh_ui";
import { Types } from "komodo_client";
import { useMutation } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import DiscoveryFields, { TunnelSelMap } from "./ingress-fields";

/** Kinds with a structured form. Any other kind falls back to raw JSON. */
const STRUCTURED_KIND = "cloudflare";

function initTunnels(params: unknown): TunnelSelMap {
  const list = (params as { tunnels?: unknown })?.tunnels;
  if (!Array.isArray(list)) return {};
  return Object.fromEntries(
    list
      .filter((t) => t?.tunnel_id)
      .map((t) => [
        String(t.tunnel_id),
        {
          account_id: String(t.account_id ?? ""),
          name: String(t.name ?? ""),
          server: String(t.server ?? ""),
        },
      ]),
  );
}

function errorMessage(e: unknown): string {
  const err = (e as { result?: { error?: string } })?.result?.error;
  return err ?? "Request failed. See console.";
}

/** Create (no `account`) or edit (with `account`) a Domain account. */
export default function DomainAccountForm({
  account,
}: {
  account?: Types.DomainAccount;
}) {
  const editing = !!account;
  const [opened, { open, close }] = useDisclosure();
  const kinds = useRead(
    "ListDomainProviderKinds",
    {},
    { enabled: opened },
  ).data;

  const [name, setName] = useState(account?.name ?? "");
  const [kind, setKind] = useState(account?.kind ?? "");
  const [token, setToken] = useState("");
  const [params, setParams] = useState(
    JSON.stringify(account?.params ?? {}, undefined, 2),
  );
  const initialParams = account?.params as { zones?: string[] } | undefined;
  const [zones, setZones] = useState<string[]>(
    Array.isArray(initialParams?.zones) ? initialParams.zones : [],
  );
  const [tunnels, setTunnels] = useState<TunnelSelMap>(
    initTunnels(account?.params),
  );
  const [discovery, setDiscovery] = useState<Types.DomainDiscovery>();
  const [discoverError, setDiscoverError] = useState<string>();
  const servers = useRead("ListServers", {}, { enabled: opened }).data ?? [];
  const structured = kind === STRUCTURED_KIND;
  const [description, setDescription] = useState(account?.description ?? "");

  const inv = useInvalidate();
  const onSuccess = () => {
    inv(["ListDomainAccounts"], ["GetDomainAccount"]);
    notifications.show({
      message: editing ? "Account updated" : "Account created",
    });
    setToken("");
    close();
  };
  const { mutate: create, isPending: createPending } = useWrite(
    "CreateDomainAccount",
    { onSuccess },
  );
  const { mutate: update, isPending: updatePending } = useWrite(
    "UpdateDomainAccount",
    { onSuccess },
  );

  // The token never goes in the query key / cache, so this is a mutation.
  const { mutate: discover, isPending: discovering } = useMutation({
    mutationFn: (req: Types.DiscoverDomainAccount) =>
      komodo_client().read("DiscoverDomainAccount", req),
    onSuccess: (d) => {
      setDiscovery(d);
      setDiscoverError(undefined);
    },
    onError: (e) => {
      console.log("Discover error:", e);
      setDiscovery(undefined);
      setDiscoverError(errorMessage(e));
    },
  });
  const canDiscover = editing || (!!token && !!kind);
  const runDiscover = () =>
    discover(
      token || !editing
        ? { kind, token }
        : { account: account.id || account.name },
    );
  // Editing: read with the saved token as soon as the modal opens.
  useEffect(() => {
    if (opened && editing && structured && account.token_set) {
      discover({ account: account.id || account.name });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [opened]);

  const buildParams = (): unknown | undefined => {
    if (!structured) {
      try {
        return JSON.parse(params || "{}");
      } catch {
        notifications.show({
          message: "Params is not valid JSON",
          color: "red",
        });
        return undefined;
      }
    }
    const entries = Object.entries(tunnels);
    // Created by one-click setup: keep the flag across edits.
    const managed = new Set(
      (
        ((account?.params as { tunnels?: unknown[] } | undefined)?.tunnels ??
          []) as { tunnel_id?: string; managed?: boolean }[]
      )
        .filter((t) => t?.managed)
        .map((t) => t.tunnel_id),
    );
    const names = entries.map(([, t]) => t.name.trim());
    if (names.some((n) => !n) || new Set(names).size !== names.length) {
      notifications.show({
        message: "Enabled tunnels need a unique local name",
        color: "red",
      });
      return undefined;
    }
    return {
      ...(account?.params as object | undefined),
      zones,
      tunnels: entries.map(([tunnel_id, t]) => ({
        name: t.name.trim(),
        account_id: t.account_id,
        tunnel_id,
        ...(t.server ? { server: t.server } : {}),
        ...(managed.has(tunnel_id) ? { managed: true } : {}),
      })),
    };
  };

  const submit = () => {
    const parsed = buildParams();
    if (parsed === undefined) return;
    if (editing) {
      update({
        id: account.id,
        // Empty token means "do not change"
        token: token || undefined,
        params: parsed,
        description,
      });
    } else {
      if (!name.trim() || !kind || !token) {
        notifications.show({
          message: "Name, kind and token are required",
          color: "red",
        });
        return;
      }
      create({ name: name.trim(), kind, token, params: parsed, description });
    }
  };

  return (
    <>
      <Modal
        opened={opened}
        onClose={close}
        title={
          <Text size="lg">
            {editing ? `Edit ${account.name}` : "Create Domain Account"}
          </Text>
        }
        size="lg"
      >
        <Stack>
          <TextInput
            label="Name"
            placeholder="cloudflare-main"
            value={name}
            onChange={(e) => setName(e.target.value)}
            disabled={editing}
          />
          <Select
            label="Kind"
            value={kind || null}
            onChange={(k) => setKind(k ?? "")}
            data={kinds?.map((k) => k.kind) ?? (kind ? [kind] : [])}
            disabled={editing}
          />
          <PasswordInput
            label="Token"
            description={
              editing
                ? `Currently ${account.token_set ? "set" : "not set"}. Leave empty to keep it unchanged.`
                : "Provider API token. Never shown again after saving."
            }
            placeholder={editing ? "Unchanged" : "API token"}
            value={token}
            onChange={(e) => setToken(e.target.value)}
            autoComplete="new-password"
          />
          {structured ? (
            <Stack gap="sm">
              <Group>
                <Button
                  size="xs"
                  variant="default"
                  onClick={runDiscover}
                  loading={discovering}
                  disabled={!canDiscover}
                >
                  Read from Cloudflare
                </Button>
                <Text size="xs" c="dimmed">
                  {editing && !token
                    ? "Uses the saved token."
                    : "Uses the token above, nothing is saved."}
                </Text>
              </Group>
              {discoverError && (
                <Alert color="red" p="xs">
                  {discoverError} You can still save and use DNS only.
                </Alert>
              )}
              <DiscoveryFields
                discovery={discovery}
                zones={zones}
                setZones={setZones}
                tunnels={tunnels}
                setTunnels={setTunnels}
                servers={servers}
              />
            </Stack>
          ) : (
            kind && (
              <Accordion variant="contained">
                <Accordion.Item value="advanced">
                  <Accordion.Control>Advanced: params (JSON)</Accordion.Control>
                  <Accordion.Panel>
                    <MonacoEditor
                      value={params}
                      language="json"
                      onValueChange={setParams}
                    />
                  </Accordion.Panel>
                </Accordion.Item>
              </Accordion>
            )
          )}
          <TextInput
            label="Description"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
          <Group justify="end">
            <Button
              leftSection={<CircleCheckBig size="1rem" />}
              onClick={submit}
              loading={createPending || updatePending}
            >
              {editing ? "Save" : "Create"}
            </Button>
          </Group>
        </Stack>
      </Modal>

      <Button
        leftSection={
          editing ? <ICONS.Edit size="1rem" /> : <ICONS.Create size="1rem" />
        }
        variant={editing ? "default" : "filled"}
        onClick={open}
      >
        {editing ? "Edit" : "New Account"}
      </Button>
    </>
  );
}
