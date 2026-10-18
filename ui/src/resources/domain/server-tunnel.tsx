import { useInvalidate, useRead, useUser, useWrite } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import ResourceLink from "@/resources/link";
import {
  Alert,
  Button,
  Checkbox,
  Group,
  Loader,
  Modal,
  Select,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { Types } from "komodo_client";
import { Section, StatusBadge } from "mogh_ui";
import { useEffect, useState } from "react";

type State = Types.ServerTunnelState;
const S = Types.ServerTunnelState;

const STATE_TEXT: Record<State, string> = {
  [S.NotEnabled]: "未开通",
  [S.Starting]: "开通中",
  [S.Healthy]: "healthy",
  [S.Degraded]: "部分连接",
  [S.Down]: "down",
  [S.Inactive]: "未连接",
};

const STATE_INTENT: Record<State, "Good" | "Warning" | "Critical" | "Unknown" | "None"> = {
  [S.NotEnabled]: "None",
  [S.Starting]: "Warning",
  [S.Healthy]: "Good",
  [S.Degraded]: "Warning",
  [S.Down]: "Critical",
  [S.Inactive]: "Unknown",
};

/** Tunnel of a Server. Polls quickly while the connector is coming up. */
export function useServerTunnel(server: string, enabled = true) {
  return useRead(
    "GetServerTunnel",
    { server },
    {
      enabled: enabled && !!server,
      refetchInterval: (q) =>
        q.state.data?.state === S.Starting ? 3_000 : 30_000,
    },
  );
}

/**
 * Confirm + progress for "open a tunnel for this server".
 * `onEnabled` fires as soon as the tunnel is registered, the connector may
 * still be connecting after that.
 */
export function EnableTunnelModal({
  server,
  serverName,
  onClose,
  onEnabled,
}: {
  server: string;
  serverName?: string;
  onClose: () => void;
  onEnabled?: () => void;
}) {
  const inv = useInvalidate();
  const tunnel = useServerTunnel(server);
  const [image, setImage] = useState<string>();
  const [account, setAccount] = useState<string | null>(null);
  const [started, setStarted] = useState(false);

  const data = tunnel.data;
  const accounts = data?.accounts ?? [];
  const chosen = account ?? data?.account ?? null;
  // Pre-fill the remembered / default image once.
  useEffect(() => {
    if (image === undefined && data) setImage(data.image);
  }, [data, image]);

  const { mutate, isPending } = useWrite("EnableServerTunnel", {
    onSuccess: () => {
      setStarted(true);
      inv(["GetServerTunnel"], ["ListDomainAccounts"], ["ListDeployments"]);
      onEnabled?.();
    },
  });

  if (started) {
    const state = data?.state ?? S.Starting;
    return (
      <Modal opened onClose={onClose} title={<b>开通隧道</b>}>
        <Stack>
          <Group gap="xs">
            {state !== S.Healthy && state !== S.Degraded && <Loader size="xs" />}
            <Text>
              {state === S.Healthy
                ? "隧道已连通，现在可以给这台服务器上的服务添加域名了。"
                : state === S.Down || state === S.Inactive
                  ? "隧道已创建，cloudflared 还没连上。镜像拉取可能需要一点时间，稍后会自动刷新。"
                  : "隧道已创建，正在启动 cloudflared 并连接 Cloudflare…"}
            </Text>
          </Group>
          {data?.warning && <Alert color="yellow">{data.warning}</Alert>}
          <Group justify="flex-end">
            <Button onClick={onClose}>
              {state === S.Healthy ? "完成" : "后台继续"}
            </Button>
          </Group>
        </Stack>
      </Modal>
    );
  }

  const needAccount = accounts.length > 1 && !chosen;
  return (
    <Modal
      opened
      onClose={onClose}
      title={<b>为{serverName ? ` ${serverName} ` : "这台服务器"}开通隧道</b>}
    >
      <Stack>
        <Text fz="sm">将依次完成：</Text>
        <Stack gap={2} fz="sm">
          <Text fz="sm">1. 在 Cloudflare 创建一条隧道</Text>
          <Text fz="sm">
            2. 在这台服务器上部署 cloudflared（Deployment{" "}
            <b>cloudflared-{serverName ?? "…"}</b>，host 网络）
          </Text>
          <Text fz="sm">3. 把隧道绑定到这台服务器，之后加域名会自动走隧道</Text>
        </Stack>
        {accounts.length > 1 && (
          <Select
            label="Cloudflare 账号"
            placeholder="选择账号"
            data={accounts}
            value={chosen}
            onChange={setAccount}
          />
        )}
        <TextInput
          label="cloudflared 镜像"
          description="服务器访问不了 Docker Hub 时，换成镜像源地址。会记在账号上，下次自动带出。"
          value={image ?? ""}
          onChange={(e) => setImage(e.currentTarget.value)}
          placeholder="cloudflare/cloudflared:latest"
          disabled={!data}
        />
        {!tunnel.isLoading && accounts.length === 0 && (
          <Alert color="yellow">
            还没有可用的 Cloudflare 账号，请先到 Settings -&gt; Domains 添加。
          </Alert>
        )}
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button
            loading={isPending}
            disabled={!data || !accounts.length || needAccount || !image?.trim()}
            onClick={() =>
              mutate({
                server,
                account: chosen ?? undefined,
                image: image?.trim(),
              })
            }
          >
            开通
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

function DisableTunnelModal({
  server,
  onClose,
}: {
  server: string;
  onClose: () => void;
}) {
  const inv = useInvalidate();
  const [deleteRemote, setDeleteRemote] = useState(true);
  const { mutate, isPending } = useWrite("DisableServerTunnel", {
    onSuccess: () => {
      inv(["GetServerTunnel"], ["ListDomainAccounts"], ["ListDeployments"]);
      notifications.show({ message: "已关闭隧道", color: "green" });
      onClose();
    },
  });
  return (
    <Modal opened onClose={onClose} title={<b>关闭隧道</b>}>
      <Stack>
        <Text fz="sm">
          会删除这台服务器上的 cloudflared，并解除隧道绑定。如果还有域名在用这条隧道，需要先删掉它们。
        </Text>
        <Checkbox
          label="同时删除 Cloudflare 上的隧道"
          checked={deleteRemote}
          onChange={(e) => setDeleteRemote(e.currentTarget.checked)}
        />
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button
            color="red"
            loading={isPending}
            onClick={() => mutate({ server, delete_remote: deleteRemote })}
          >
            关闭隧道
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

/** "Cloudflare Tunnel" block on a Server page. Admin only. */
export function ServerTunnelSection({ id }: { id: string }) {
  const isAdmin = useUser().data?.admin ?? false;
  const tunnel = useServerTunnel(id, isAdmin);
  const server = useRead("ListServers", {}, { enabled: isAdmin }).data?.find(
    (s) => s.id === id,
  );
  const [modal, setModal] = useState<"enable" | "disable">();
  if (!isAdmin || !tunnel.data) return null;
  const t = tunnel.data;
  const enabled = t.state !== S.NotEnabled;
  return (
    <Section
      title="Cloudflare Tunnel"
      icon={<ICONS.Domain size="1.3rem" />}
      description="让这台服务器上的服务可以从外网访问，不用开放端口。"
      actions={
        enabled ? (
          t.managed ? (
            <Button
              variant="default"
              color="red"
              onClick={() => setModal("disable")}
            >
              关闭隧道
            </Button>
          ) : undefined
        ) : (
          <Button
            leftSection={<ICONS.Create size="1rem" />}
            disabled={!t.accounts.length}
            onClick={() => setModal("enable")}
          >
            开通隧道
          </Button>
        )
      }
    >
      <Stack gap="xs">
        <Group gap="xs">
          <StatusBadge text={STATE_TEXT[t.state]} intent={STATE_INTENT[t.state]} />
          {t.tunnel_name && <Text fz="sm">{t.tunnel_name}</Text>}
          {t.account && enabled && (
            <Text fz="sm" c="dimmed">
              账号 {t.account}
            </Text>
          )}
        </Group>
        {t.deployment_id && (
          <Group gap="xs">
            <Text fz="sm" c="dimmed">
              cloudflared：
            </Text>
            <ResourceLink type="Deployment" id={t.deployment_id} />
            {t.deployment_state && (
              <Text fz="sm" c="dimmed">
                {t.deployment_state}
              </Text>
            )}
          </Group>
        )}
        {!enabled && !t.accounts.length && (
          <Text c="dimmed" fz="sm">
            还没有 Cloudflare 账号，请先到 Settings -&gt; Domains 添加。
          </Text>
        )}
        {!enabled && !!t.accounts.length && (
          <Text c="dimmed" fz="sm">
            还没有隧道。点「开通隧道」即可，大约几十秒。
          </Text>
        )}
        {enabled && !t.managed && (
          <Text c="dimmed" fz="sm">
            这条隧道是手动绑定的，如需解绑请到 Settings -&gt; Domains 修改账号。
          </Text>
        )}
        {enabled && !t.deployment_id && !t.warning && (
          <Text c="dimmed" fz="sm">
            隧道由其他方式运行。
          </Text>
        )}
        {t.warning && (
          <Alert color="yellow" p="xs">
            {t.warning}
            {t.managed && !t.deployment_id && (
              <Button
                ml="xs"
                size="compact-xs"
                variant="light"
                onClick={() => setModal("enable")}
              >
                继续开通
              </Button>
            )}
          </Alert>
        )}
        {!!t.domains.length && (
          <Text fz="xs" c="dimmed">
            正在使用：{t.domains.join("、")}
          </Text>
        )}
      </Stack>
      {modal === "enable" && (
        <EnableTunnelModal
          server={id}
          serverName={server?.name}
          onClose={() => setModal(undefined)}
        />
      )}
      {modal === "disable" && (
        <DisableTunnelModal server={id} onClose={() => setModal(undefined)} />
      )}
    </Section>
  );
}
