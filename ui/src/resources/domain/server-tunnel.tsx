import { useInvalidate, useRead, useUser, useWrite } from "@/lib/hooks";
import { Button, Group, Modal, Stack, Text } from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { Types } from "komodo_client";
import { Section, StatusBadge } from "mogh_ui";
import { ICONS } from "@/lib/icons";
import { errorText } from "./make-public";

const S = Types.ServerTunnelState;

const STATE_TEXT: Record<Types.ServerTunnelState, string> = {
  [S.NotEnabled]: "off",
  [S.Starting]: "connecting",
  [S.Healthy]: "connected (healthy)",
  [S.Degraded]: "connected (degraded)",
  [S.Down]: "down",
  [S.Inactive]: "not connected",
};

const STATE_INTENT: Record<
  Types.ServerTunnelState,
  "Good" | "Warning" | "Critical" | "Unknown" | "None"
> = {
  [S.NotEnabled]: "None",
  [S.Starting]: "Warning",
  [S.Healthy]: "Good",
  [S.Degraded]: "Warning",
  [S.Down]: "Critical",
  [S.Inactive]: "Unknown",
};

/** One line on a Server page. The connection opens by itself on first use. */
export function ServerTunnelSection({ id }: { id: string }) {
  const isAdmin = useUser().data?.admin ?? false;
  const inv = useInvalidate();
  const [opened, { open, close }] = useDisclosure();
  const t = useRead(
    "GetServerTunnel",
    { server: id },
    {
      enabled: isAdmin,
      refetchInterval: (q) =>
        q.state.data?.state === S.Starting ? 3_000 : 30_000,
    },
  ).data;
  const { mutate, isPending } = useWrite("DisableServerTunnel", {
    onSuccess: () => {
      inv(["GetServerTunnel"], ["ListDomainAccounts"], ["ListServices"]);
      notifications.show({ message: "Public access turned off" });
      close();
    },
    onError: (e) =>
      notifications.show({ message: errorText(e), color: "red" }),
  });
  if (!isAdmin || !t || t.state === S.NotEnabled) return null;
  return (
    <Section title="Public access" icon={<ICONS.Domain size="1.3rem" />}>
      <Group justify="space-between">
        <Group gap="xs">
          <Text>Public access:</Text>
          <StatusBadge text={STATE_TEXT[t.state]} intent={STATE_INTENT[t.state]} />
          {t.warning && (
            <Text fz="sm" c="dimmed">
              {t.warning}
            </Text>
          )}
        </Group>
        {t.managed && (
          <Button variant="default" size="compact-sm" onClick={open}>
            Disable
          </Button>
        )}
      </Group>
      <Modal opened={opened} onClose={close} title={<b>Disable public access</b>}>
        <Stack>
          <Text fz="sm">
            Removes the connection from this server. Stop public access of its
            services first.
          </Text>
          <Group justify="flex-end">
            <Button variant="default" onClick={close}>
              Cancel
            </Button>
            <Button
              color="red"
              loading={isPending}
              onClick={() => mutate({ server: id, delete_remote: true })}
            >
              Disable
            </Button>
          </Group>
        </Stack>
      </Modal>
    </Section>
  );
}
