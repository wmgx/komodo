import { useInvalidate, useRead, useUser, useWrite } from "@/lib/hooks";
import {
  Alert,
  Anchor,
  Button,
  Code,
  Group,
  Loader,
  Modal,
  Select,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { Check, Circle, Globe, X } from "lucide-react";
import { Types } from "komodo_client";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useZones } from "./service-hooks";
import { toLabel, validatePrefix } from "./text";

const WEB_PORTS = [80, 8080, 3000, 9120, 8000, 8888, 443, 5000];

/** Most HTTP-looking port, else the first. */
export function pickPort(ports: Types.ServicePort[]): number | undefined {
  const web = WEB_PORTS.find((w) => ports.some((p) => p.host === w));
  return web ?? ports[0]?.host;
}

export const sameService = (a: Types.ServiceItem, b: Types.ServiceItem) =>
  a.target.type === b.target.type &&
  a.target.id === b.target.id &&
  a.service === b.service;

/** Error text of a failed request. */
export const errorText = (e: unknown) =>
  (e as { result?: { error?: string } })?.result?.error ??
  "Something went wrong. Check the Core logs.";

/** "Make public" or "Stop public access", whichever applies. Admins only. */
export function ServiceAccessButton({ item }: { item: Types.ServiceItem }) {
  const isAdmin = useUser().data?.admin ?? false;
  if (!isAdmin || !item.server_id) return null;
  return item.public.length ? (
    <StopPublicButton item={item} />
  ) : (
    <MakePublicButton item={item} />
  );
}

function MakePublicButton({ item }: { item: Types.ServiceItem }) {
  const [opened, { open, close }] = useDisclosure();
  return (
    <>
      <Button
        size="compact-sm"
        leftSection={<Globe size="1rem" />}
        onClick={open}
        loading={!!item.expose}
      >
        Make public
      </Button>
      {opened && <MakePublicModal item={item} onClose={close} />}
    </>
  );
}

function StopPublicButton({ item }: { item: Types.ServiceItem }) {
  const inv = useInvalidate();
  const [opened, { open, close }] = useDisclosure();
  const { mutate, isPending } = useWrite("UnexposeService", {
    onSuccess: () => {
      inv(["ListServices"], ["ListServiceDomains"], ["ListDomains"]);
      notifications.show({ message: "Public access stopped" });
      close();
    },
    onError: (e) =>
      notifications.show({ message: errorText(e), color: "red" }),
  });
  return (
    <>
      <Button
        size="compact-sm"
        variant="default"
        leftSection={<X size="1rem" />}
        onClick={open}
      >
        Stop public access
      </Button>
      <Modal opened={opened} onClose={close} title={<b>Stop public access</b>}>
        <Stack>
          <Text fz="sm">
            {item.public.map((p) => p.url).join(", ")} will stop working. The
            service itself keeps running.
          </Text>
          <Group justify="flex-end">
            <Button variant="default" onClick={close}>
              Cancel
            </Button>
            <Button
              color="red"
              loading={isPending}
              onClick={() =>
                mutate({ target: item.target, service: item.service })
              }
            >
              Stop public access
            </Button>
          </Group>
        </Stack>
      </Modal>
    </>
  );
}

type Phase = "form" | "running" | "done" | "conflicts";

function MakePublicModal({
  item,
  onClose,
}: {
  item: Types.ServiceItem;
  onClose: () => void;
}) {
  const inv = useInvalidate();
  const [prefix, setPrefix] = useState(toLabel(item.name));
  const [zone, setZone] = useState<string | null>(null);
  const [port, setPort] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>("form");
  const [conflicts, setConflicts] = useState<Types.Change[]>([]);
  const [url, setUrl] = useState<string>();
  const [problem, setProblem] = useState<string>();

  const { zones, loading, failed } = useZones(true);
  useEffect(() => {
    if (zones.length === 1 && !zone) setZone(zones[0].zone);
  }, [zones, zone]);
  useEffect(() => {
    if (!port && item.ports.length) setPort(String(pickPort(item.ports)));
  }, [item.ports, port]);

  // While running, the list reports which step the server is on.
  const live = useRead(
    "ListServices",
    {},
    { enabled: phase === "running", refetchInterval: 1_500 },
  ).data?.find((s) => sameService(s, item));

  const { mutate } = useWrite("ExposeService", {
    onSuccess: (res) => {
      inv(
        ["ListServices"],
        ["ListServiceDomains"],
        ["ListDomains"],
        ["GetServerTunnel"],
      );
      if (res.conflicts.length) {
        setConflicts(res.conflicts);
        setPhase("conflicts");
      } else if (res.url) {
        setUrl(res.url);
        setPhase("done");
      } else {
        setProblem(res.error ?? "Could not make the service public.");
        setPhase("form");
      }
    },
    onError: (e) => {
      setProblem(errorText(e));
      setPhase("form");
    },
  });

  const submit = (adopt_conflicts: boolean) => {
    setProblem(undefined);
    setPhase("running");
    mutate({
      target: item.target,
      service: item.service,
      prefix: prefix.trim(),
      zone: zone ?? undefined,
      port: Number(port),
      adopt_conflicts,
    });
  };

  const prefixError = validatePrefix(prefix.trim());
  const canSubmit =
    !!prefix.trim() && !prefixError && !!zone && !!port && !loading;

  const title = <b>Make {item.name} public</b>;

  if (phase === "running")
    return (
      <Modal opened onClose={onClose} title={title} closeOnClickOutside={false}>
        <Stack>
          <Step
            label="Opening a connection to the server"
            state={live?.expose === Types.ExposeStage.Configuring ? "done" : "active"}
          />
          <Step
            label="Setting up the address"
            state={
              live?.expose === Types.ExposeStage.Configuring
                ? "active"
                : "pending"
            }
          />
          <Step label="Done" state="pending" />
          <Text fz="xs" c="dimmed">
            The first service on a server takes about a minute. You can close
            this window, it keeps going.
          </Text>
        </Stack>
      </Modal>
    );

  if (phase === "done")
    return (
      <Modal opened onClose={onClose} title={title}>
        <Stack>
          <Step label="Opening a connection to the server" state="done" />
          <Step label="Setting up the address" state="done" />
          <Step label="Done" state="done" />
          <Anchor href={url} target="_blank" rel="noreferrer" fw="bold">
            {url}
          </Anchor>
          <Group justify="flex-end">
            <Button onClick={onClose}>Close</Button>
          </Group>
        </Stack>
      </Modal>
    );

  if (phase === "conflicts")
    return (
      <Modal opened onClose={onClose} title={<b>Replace existing records?</b>}>
        <Stack>
          <Text fz="sm">
            These already exist and are not managed by Komodo. Continuing
            replaces them:
          </Text>
          {conflicts.map((c, i) => (
            <Text key={i} fz="sm">
              <Code>{c.object.key}</Code>
              {c.before ? <Code ml="xs">{JSON.stringify(c.before)}</Code> : null}
            </Text>
          ))}
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setPhase("form")}>
              Back
            </Button>
            <Button color="red" onClick={() => submit(true)}>
              Replace and continue
            </Button>
          </Group>
        </Stack>
      </Modal>
    );

  return (
    <Modal opened onClose={onClose} title={title}>
      <Stack>
        <Group align="flex-start" grow wrap="nowrap">
          <TextInput
            label="Address"
            value={prefix}
            onChange={(e) => setPrefix(e.currentTarget.value)}
            error={prefixError}
            rightSectionWidth={zones.length === 1 ? undefined : 0}
            rightSection={
              zones.length === 1 ? (
                <Text fz="sm" pr="xs" c="dimmed">
                  .{zones[0].zone}
                </Text>
              ) : undefined
            }
            rightSectionPointerEvents="none"
            styles={zones.length === 1 ? { input: { paddingRight: "9rem" } } : undefined}
            data-autofocus
          />
          {zones.length > 1 && (
            <Select
              label="Domain"
              placeholder="Choose"
              value={zone}
              onChange={setZone}
              data={zones.map((z) => z.zone)}
            />
          )}
        </Group>
        {item.ports.length > 1 && (
          <Select
            label="Port"
            value={port}
            onChange={setPort}
            allowDeselect={false}
            data={item.ports.map((p) => ({
              value: String(p.host),
              label: `${p.host} -> ${p.container}`,
            }))}
          />
        )}
        {!item.ports.length && (
          <Alert color="yellow" p="xs">
            No published port found. Start the service with a published port
            first.
          </Alert>
        )}
        {!loading && !zones.length && (
          <Alert color="yellow" p="xs">
            No domain available yet. Add a Cloudflare account in{" "}
            <Anchor component={Link} to="/settings" onClick={onClose}>
              Settings
            </Anchor>{" "}
            first.
          </Alert>
        )}
        {failed && (
          <Text fz="xs" c="red">
            Could not read the domains of an account. Check its token in
            Settings.
          </Text>
        )}
        {problem && <Alert color="red">{problem}</Alert>}
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={!canSubmit} onClick={() => submit(false)}>
            Make public
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

function Step({
  label,
  state,
}: {
  label: string;
  state: "pending" | "active" | "done";
}) {
  return (
    <Group gap="xs" wrap="nowrap" c={state === "pending" ? "dimmed" : undefined}>
      {state === "done" ? (
        <Check size="1rem" color="var(--mantine-color-green-6)" />
      ) : state === "active" ? (
        <Loader size="1rem" />
      ) : (
        <Circle size="1rem" />
      )}
      <Text fz="sm">{label}</Text>
    </Group>
  );
}
