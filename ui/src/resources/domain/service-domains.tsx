import { useInvalidate, usePermissions, useRead, useUser, useWrite } from "@/lib/hooks";
import { ICONS } from "@/lib/icons";
import { domainStatusIntention } from "@/lib/color";
import { Anchor, Badge, Group, Stack, Text } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { ConfirmModal, Section, StatusBadge } from "mogh_ui";
import { Types } from "komodo_client";
import AddDomain from "./add-domain";
import { ServiceTarget } from "./service-hooks";
import { domainLink, reasonText, statusReasons } from "./text";

function Row({ domain }: { domain: Types.DomainListItem }) {
  const inv = useInvalidate();
  const full = useRead(
    "GetDomain",
    { domain: domain.id },
    { refetchInterval: 30_000 },
  ).data;
  const { mutateAsync, isPending } = useWrite("DeleteDomain", {
    onSuccess: () => {
      inv(["ListServiceDomains"], ["ListDomains"]);
      notifications.show({ message: `已删除 ${domain.info.hostname}` });
    },
  });
  const tunnel = !!domain.info.ingress_account;
  const link = full?.config ? domainLink(full.config!) : `https://${domain.info.hostname}`;
  const reasons = statusReasons(full?.info?.status);
  const status = domain.info.status_code;
  return (
    <Group justify="space-between" wrap="nowrap" align="flex-start">
      <Stack gap={2}>
        <Group gap="xs">
          <Anchor href={link} target="_blank" rel="noreferrer">
            {link}
          </Anchor>
          <Badge variant="default">{tunnel ? "Tunnel" : "DNS"}</Badge>
          {domain.info.service && (
            <Badge variant="light">{domain.info.service}</Badge>
          )}
          <StatusBadge
            text={status === "unchecked" ? "unchecked" : status}
            intent={domainStatusIntention(status)}
          />
        </Group>
        {reasons.map((r, i) => (
          <Text key={i} fz="xs" c="dimmed">
            {reasonText(r)}
          </Text>
        ))}
        {status === "unchecked" && (
          <Text fz="xs" c="dimmed">
            尚未检查，打开 Domain 页面点 Preview plan 可查看状态。
          </Text>
        )}
      </Stack>
      <ConfirmModal
        title={
          <>
            删除 <b>{domain.info.hostname}</b>
          </>
        }
        confirmButtonContent="Delete"
        icon={<ICONS.Delete size="1rem" />}
        targetNoIcon
        targetProps={{ w: "fit", px: "xs", variant: "default" }}
        confirmText={domain.name}
        onConfirm={() => mutateAsync({ id: domain.id })}
        loading={isPending}
        confirmProps={{ variant: "filled", color: "red" }}
      >
        <ICONS.Delete size="1rem" />
      </ConfirmModal>
    </Group>
  );
}

/** "Domains" block on a Stack / Deployment page. */
export default function ServiceDomains({ target }: { target: ServiceTarget }) {
  const isAdmin = useUser().data?.admin ?? false;
  const { canWrite } = usePermissions({ type: target.type, id: target.id });
  const domains = useRead("ListServiceDomains", { target }).data ?? [];
  if (!isAdmin && !domains.length) return null;
  return (
    <Section
      title="Domains"
      icon={<ICONS.Domain size="1.3rem" />}
      description="给这个服务注册域名，直接用域名访问。"
      actions={isAdmin && canWrite ? <AddDomain target={target} /> : undefined}
    >
      {domains.length ? (
        <Stack>
          {domains.map((d) => (
            <Row key={d.id} domain={d} />
          ))}
        </Stack>
      ) : (
        <Text c="dimmed">还没有域名。点 Add domain 即可，通常只需输入主机名。</Text>
      )}
    </Section>
  );
}

export const StackDomains = ({ id }: { id: string }) => (
  <ServiceDomains target={{ type: "Stack", id }} />
);
export const DeploymentDomains = ({ id }: { id: string }) => (
  <ServiceDomains target={{ type: "Deployment", id }} />
);
