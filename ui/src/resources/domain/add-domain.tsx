import { useInvalidate, useWrite } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import {
  Alert,
  Autocomplete,
  Button,
  Code,
  Collapse,
  Group,
  Modal,
  SegmentedControl,
  Select,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import { notifications } from "@mantine/notifications";
import { Types } from "komodo_client";
import { useEffect, useState } from "react";
import {
  ServiceTarget,
  useAccessHint,
  useServicePorts,
  useZones,
} from "./service-hooks";
import { reasonText, validatePrefix } from "./text";

export default function AddDomain({ target }: { target: ServiceTarget }) {
  const [opened, { open, close }] = useDisclosure();
  return (
    <>
      <Button leftSection={<ICONS.Create size="1rem" />} onClick={open}>
        Add domain
      </Button>
      {opened && <AddDomainModal target={target} onClose={close} />}
    </>
  );
}

function AddDomainModal({
  target,
  onClose,
}: {
  target: ServiceTarget;
  onClose: () => void;
}) {
  const inv = useInvalidate();
  const [prefix, setPrefix] = useState("");
  const [zone, setZone] = useState<string | null>(null);
  const [service, setService] = useState("");
  const [portText, setPortText] = useState("");
  const [mode, setMode] = useState<Types.DomainMode>(Types.DomainMode.Auto);
  const [showAdvanced, { toggle: toggleAdvanced }] = useDisclosure();
  const [conflicts, setConflicts] = useState<Types.Change[]>([]);
  const [problem, setProblem] = useState<string>();

  const { zones, accounts, loading, failed } = useZones(true);
  const { serviceNames, ports } = useServicePorts(target, service);
  const hint = useAccessHint(target, accounts);

  // One choice -> take it.
  useEffect(() => {
    if (zones.length === 1 && !zone) setZone(zones[0].zone);
  }, [zones, zone]);
  useEffect(() => {
    if (target.type === "Stack" && serviceNames.length === 1 && !service)
      setService(serviceNames[0]);
  }, [serviceNames, service, target.type]);
  useEffect(() => {
    if (ports.length === 1 && !portText) setPortText(String(ports[0]));
  }, [ports, portText]);

  const prefixError = validatePrefix(prefix.trim());
  const port = Number(portText);
  const portOk = Number.isInteger(port) && port > 0 && port < 65536;
  const hostname = zone ? (prefix.trim() ? `${prefix.trim()}.${zone}` : zone) : "";
  const canSubmit =
    !!zone &&
    !prefixError &&
    portOk &&
    (target.type === "Deployment" || !!service.trim());

  const { mutate, isPending } = useWrite("AddServiceDomain", {
    onSuccess: (res) => {
      setProblem(undefined);
      if (res.conflicts.length) {
        setConflicts(res.conflicts);
        return;
      }
      if (res.domain) {
        inv(["ListServiceDomains"], ["ListDomains"], ["GetDomain"]);
        if (res.error) {
          notifications.show({
            title: "域名已创建，但生效失败",
            message: res.reason ? reasonText(res.reason) : res.error,
            color: "red",
          });
        } else {
          notifications.show({
            message: `已添加 ${res.domain.config?.hostname}`,
            color: "green",
          });
        }
        onClose();
        return;
      }
      setProblem(res.reason ? reasonText(res.reason) : res.error);
    },
  });

  const submit = (adopt_conflicts: boolean) =>
    mutate({
      target,
      service: target.type === "Stack" ? service.trim() : "",
      hostname,
      port,
      mode,
      adopt_conflicts,
    });

  return (
    <>
      <Modal
        opened={!conflicts.length}
        onClose={onClose}
        title={<b>Add domain</b>}
        size="lg"
      >
        <Stack>
          <Group align="flex-start" grow wrap="nowrap">
            <TextInput
              label="Hostname"
              placeholder="app"
              value={prefix}
              onChange={(e) => setPrefix(e.currentTarget.value)}
              error={prefixError}
              data-autofocus
            />
            <Select
              label="Zone"
              placeholder={loading ? "Loading..." : "Select zone"}
              value={zone}
              onChange={setZone}
              searchable
              data={zones.map((z) => ({
                value: z.zone,
                label: `${z.zone}  (${z.accounts.join(", ")})`,
              }))}
              nothingFoundMessage="没有可用的 zone，请先到 Settings -> Domains 添加账号"
            />
          </Group>
          {failed && (
            <Text fz="xs" c="red">
              部分账号读取 zone 失败，请检查 token。
            </Text>
          )}

          {target.type === "Stack" && (
            <Autocomplete
              label="Service"
              placeholder="compose 里的服务名"
              value={service}
              onChange={setService}
              data={serviceNames}
            />
          )}
          <Autocomplete
            label="Port"
            description={
              ports.length ? "已从已发布端口读取" : "没有读到已发布端口，请手动输入"
            }
            placeholder="8080"
            value={portText}
            onChange={setPortText}
            data={ports.map(String)}
            error={portText && !portOk ? "端口应为 1-65535" : undefined}
          />

          <AccessHint hint={hint} mode={mode} hostname={hostname} port={portText} />

          <Button variant="subtle" size="compact-sm" onClick={toggleAdvanced}>
            {showAdvanced ? "收起高级选项" : "高级选项"}
          </Button>
          <Collapse expanded={showAdvanced}>
            <Stack gap="xs">
              <Text fz="sm">接入方式（默认自动）</Text>
              <SegmentedControl
                value={mode}
                onChange={(v) => setMode(v as Types.DomainMode)}
                data={[
                  { value: Types.DomainMode.Auto, label: "自动" },
                  { value: Types.DomainMode.Tunnel, label: "Tunnel" },
                  { value: Types.DomainMode.Dns, label: "DNS" },
                ]}
              />
            </Stack>
          </Collapse>

          {problem && <Alert color="red">{problem}</Alert>}

          <Group justify="flex-end">
            <Button variant="default" onClick={onClose}>
              Cancel
            </Button>
            <Button
              loading={isPending}
              disabled={!canSubmit}
              onClick={() => submit(false)}
            >
              Add
            </Button>
          </Group>
        </Stack>
      </Modal>

      <Modal
        opened={!!conflicts.length}
        onClose={() => setConflicts([])}
        title={<b>覆盖远端已有的记录？</b>}
      >
        <Stack>
          <Text fz="sm">
            以下对象已存在，但不归 Komodo 管理。继续将接管并覆盖它们：
          </Text>
          {conflicts.map((c, i) => (
            <Text key={i} fz="sm">
              {c.object.kind === "dns_record" ? "DNS 记录" : "隧道规则"}{" "}
              <Code>{c.object.key}</Code>
              {c.before ? <Code ml="xs">{JSON.stringify(c.before)}</Code> : null}
            </Text>
          ))}
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setConflicts([])}>
              返回
            </Button>
            <Button
              color="red"
              loading={isPending}
              onClick={() => submit(true)}
            >
              覆盖并添加
            </Button>
          </Group>
        </Stack>
      </Modal>
    </>
  );
}

function AccessHint({
  hint,
  mode,
  hostname,
  port,
}: {
  hint: ReturnType<typeof useAccessHint>;
  mode: Types.DomainMode;
  hostname: string;
  port: string;
}) {
  const url = hostname ? hostname : "域名";
  if (mode !== Types.DomainMode.Dns && hint.tunnel)
    return (
      <Alert color="green" p="xs">
        将通过服务器 {hint.serverName} 上的隧道访问，{url} 可从外网打开。
      </Alert>
    );
  if (mode === Types.DomainMode.Tunnel)
    return (
      <Alert color="yellow" p="xs">
        这台服务器还没有隧道。想从外网访问，请先给它开通隧道（即将支持一键开通），或在 Settings -&gt; Domains 里把已有隧道绑定到该服务器。
      </Alert>
    );
  if (hint.lanOnly)
    return (
      <Alert color="yellow" p="xs">
        将解析到内网地址 {hint.host}，只能在局域网访问。想从外网访问，需要先给这台服务器开通隧道。
      </Alert>
    );
  if (hint.unknown)
    return (
      <Alert color="yellow" p="xs">
        这台服务器没有隧道，也没有对外地址。请先在服务器设置里填写 External Address，或给它开通隧道。
      </Alert>
    );
  return (
    <Alert color="blue" p="xs">
      将解析到服务器地址 {hint.host}，通过 http://{url}:{port || "端口"} 访问。
    </Alert>
  );
}
